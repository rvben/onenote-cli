# Windows PowerShell 5.1; fixed code, JSON request on stdin, no user code evaluation.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$app = $null
$failed = $false
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    if ($request.operation -notin @('probe', 'notebooks', 'sections', 'pages', 'search', 'read', 'scan')) {
        throw [ArgumentException]::new('Unknown OneNote operation.')
    }
    $app = New-Object -ComObject OneNote.Application
    [string]$xml = ''
    # xs2013 = 2; use a fixed schema version on every call.
    switch ($request.operation) {
        { $_ -in 'probe', 'notebooks' } { $app.GetHierarchy('', 2, [ref]$xml, 2) }
        'sections' { $app.GetHierarchy([string]$request.scope, 3, [ref]$xml, 2) }
        'pages' { $app.GetHierarchy([string]$request.scope, 4, [ref]$xml, 2) }
        'search' {
            $searchService = Get-Service -Name WSearch -ErrorAction SilentlyContinue
            if ($null -eq $searchService -or $searchService.Status -ne 'Running') {
                throw 'OneNote search requires the Windows Search (WSearch) service. Start Windows Search and retry. Notebook listing and page reading do not require it.'
            }
            # Include unindexed metadata so Rust can report incomplete indexing;
            # those pages are NOT necessarily matches and must be filtered there.
            $app.FindPages([string]$request.scope, [string]$request.query, [ref]$xml, $true, $false, 2)
        }
        'scan' {
            if ($request.scan_limit -lt 1 -or $request.scan_limit -gt 500 -or $request.scan_offset -lt 0 -or $request.scan_offset -gt 1000000) {
                throw [ArgumentException]::new('Invalid scan bounds.')
            }
            $clock = [Diagnostics.Stopwatch]::StartNew()
            $app.GetHierarchy([string]$request.scope, 4, [ref]$xml, 2)
            # Leave headroom for JSON escaping within the 32 MiB bridge cap.
            $budget = 4 * 1024 * 1024
            if ($xml.Length -gt $budget) { throw 'Scan hierarchy is too large; narrow --scope.' }
            $doc = New-Object Xml.XmlDocument
            $doc.XmlResolver = $null
            $settings = New-Object Xml.XmlReaderSettings
            $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
            $reader = [Xml.XmlReader]::Create((New-Object IO.StringReader($xml)), $settings)
            try { $doc.Load($reader) } finally { $reader.Dispose() }
            $ns = New-Object Xml.XmlNamespaceManager($doc.NameTable)
            $ns.AddNamespace('one', 'http://schemas.microsoft.com/office/onenote/2013/onenote')
            if ($doc.DocumentElement.NamespaceURI -ne $ns.LookupNamespace('one')) { throw 'Unexpected OneNote XML namespace.' }
            $visible = "not(ancestor-or-self::*[@isRecycleBin='true' or @isInRecycleBin='true'])"
            $locked = @($doc.SelectNodes("//one:Section[@locked='true' and $visible]", $ns)).Count
            $candidates = @($doc.SelectNodes("//one:Page[$visible and not(ancestor-or-self::*[@locked='true'])]", $ns))
            $pages = New-Object 'Collections.Generic.List[object]'
            $errors = New-Object 'Collections.Generic.List[object]'
            $position = [int]$request.scan_offset
            $attempted = 0
            $used = $xml.Length
            while ($position -lt $candidates.Count -and $attempted -lt $request.scan_limit -and $clock.Elapsed.TotalSeconds -lt 20) {
                $id = $candidates[$position].GetAttribute('ID')
                if ([string]::IsNullOrEmpty($id)) { throw 'OneNote returned a page without an ID.' }
                [string]$content = ''
                try {
                    $app.GetPageContent($id, [ref]$content, 0, 2)
                    if ($content.Length -gt $budget - $xml.Length) {
                        $errors.Add(@{id=$id;reason='page_too_large'})
                    } elseif ($used + $content.Length -gt $budget) {
                        break
                    } else {
                        $pages.Add(@{id=$id;xml=$content})
                        $used += $content.Length
                    }
                } catch {
                    $errors.Add(@{id=$id;reason='unreadable'})
                }
                $position++
                $attempted++
            }
            $next = if ($position -lt $candidates.Count) { $position } else { $null }
            $scanResult = @{xml=$xml;pages=$pages.ToArray();skipped_pages=$errors.ToArray();skipped_locked_sections=$locked;
                scanned_pages=$pages.Count;attempted_pages=$attempted;candidate_pages=$candidates.Count;
                scan_offset=[int]$request.scan_offset;next_scan_offset=$next}
        }
        'read' { $app.GetPageContent([string]$request.id, [ref]$xml, 0, 2) }
    }
    $result = if ($request.operation -eq 'scan') { $scanResult } else { @{xml=$xml} }
} catch {
    $failed = $true
    $kind = 'desktop_error'
    $message = $_.Exception.Message
    $exception = $_.Exception
    while ($null -ne $exception.InnerException) { $exception = $exception.InnerException }
    if ($exception -is [ArgumentException]) { $kind = 'invalid_input' }
    if ($null -eq $app -and $kind -ne 'invalid_input') {
        $message = 'Cannot connect to OneNote.Application. Install Windows OneNote desktop, open it under this Windows user, and complete any setup prompts. ' + $message
    } elseif ($kind -eq 'desktop_error' -and $exception.HResult -eq -2147467259) {
        $message += ' Open OneNote in the Windows desktop session, complete first-run sign-in, and open a notebook before retrying.'
    }
    if (('{0:X8}' -f $exception.HResult) -eq '8004201B') {
        $message += ' This section is password-protected. Unlock it in OneNote desktop and retry.'
    }
    $result = @{error=@{kind=$kind; message=$message}}
} finally {
    if ($null -ne $app -and [Runtime.InteropServices.Marshal]::IsComObject($app)) {
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
    }
    # Never quit the user's OneNote application.
}
$result | ConvertTo-Json -Depth 8 -Compress
if ($failed) { exit 1 }
