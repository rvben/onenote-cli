# Windows PowerShell 5.1; fixed code, JSON request on stdin, no user code evaluation.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$app = $null
$failed = $false
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    if ($request.operation -notin @('probe', 'notebooks', 'sections', 'pages', 'search', 'read')) {
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
        'read' { $app.GetPageContent([string]$request.id, [ref]$xml, 0, 2) }
    }
    $result = @{xml=$xml}
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
