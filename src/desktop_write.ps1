# Windows PowerShell 5.1; fixed code with a JSON request on stdin.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$ns = 'http://schemas.microsoft.com/office/onenote/2013/onenote'
$app = $null
$pageId = ''
$attempted = $false
$created = $false
$failed = $false
function Parse-Page([string]$xml, [string]$id) {
    $doc = New-Object Xml.XmlDocument
    $doc.XmlResolver = $null
    $settings = New-Object Xml.XmlReaderSettings
    $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
    $reader = [Xml.XmlReader]::Create((New-Object IO.StringReader($xml)), $settings)
    try { $doc.Load($reader) } finally { $reader.Dispose() }
    if ($doc.DocumentElement.LocalName -ne 'Page' -or $doc.DocumentElement.NamespaceURI -ne $ns -or $doc.DocumentElement.GetAttribute('ID') -cne $id) {
        throw 'OneNote returned a different page than requested.'
    }
    return ,$doc
}
function Nodes($node, [string]$name) {
    return $node.SelectNodes("*[local-name()='$name' and namespace-uri()='$ns']")
}
function Normalize([string]$text) { return [regex]::Replace($text, '\s+', ' ').Trim() }
function Plain($node) {
    $parts = @($node.SelectNodes(".//*[local-name()='T' and namespace-uri()='$ns']") | ForEach-Object {
        $html = [regex]::Replace($_.InnerText, '(?i)<br\s*/?>', "`n")
        [Net.WebUtility]::HtmlDecode([regex]::Replace($html, '<[^>]*>', ''))
    })
    return Normalize ($parts -join "`n")
}
function Text-Node($doc, $parent, [string]$text) {
    foreach ($line in $text.Replace("`r`n", "`n").Replace("`r", "`n").Split("`n")) {
        $oe = $doc.CreateElement('one', 'OE', $ns)
        $t = $doc.CreateElement('one', 'T', $ns)
        # OneNote drops HTML <br> during import; use a separate OE per line.
        $t.InnerText = if ($line.Length -eq 0) { '&nbsp;' } else { [Net.WebUtility]::HtmlEncode($line) }
        [void]$oe.AppendChild($t)
        [void]$parent.AppendChild($oe)
    }
}
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    if ($request.operation -notin @('create', 'append')) { throw [ArgumentException]::new('Unknown write operation.') }
    $target = [string]$request.target
    $text = [string]$request.text
    if ([string]::IsNullOrWhiteSpace($target) -or $target.Length -gt 4096 -or $target -match '[\x00-\x1f\x7f]') { throw [ArgumentException]::new('Invalid target ID.') }
    if ([string]::IsNullOrWhiteSpace($text) -or [Text.Encoding]::UTF8.GetByteCount($text) -gt 1048576) { throw [ArgumentException]::new('Content must contain 1 byte to 1 MiB of text.') }
    [void][Xml.XmlConvert]::VerifyXmlChars($text)
    if ($request.operation -eq 'create') {
        if ([string]::IsNullOrWhiteSpace($request.title) -or [Text.Encoding]::UTF8.GetByteCount([string]$request.title) -gt 512 -or $request.title -match '[\x00-\x1f\x7f]') { throw [ArgumentException]::new('Invalid title.') }
        [void][Xml.XmlConvert]::VerifyXmlChars([string]$request.title)
    }
    $app = New-Object -ComObject OneNote.Application
    if ($request.operation -eq 'create') {
        # Validate the section before the first mutating call.
        [string]$hierarchy = ''
        $app.GetHierarchy($target, 3, [ref]$hierarchy, 2)
        $sectionDoc = New-Object Xml.XmlDocument
        $sectionDoc.XmlResolver = $null
        $sectionDoc.LoadXml($hierarchy)
        $section = $sectionDoc.DocumentElement
        if ($section.LocalName -ne 'Section' -or $section.NamespaceURI -ne $ns -or $section.GetAttribute('ID') -cne $target) { throw [ArgumentException]::new('Target must be a section ID.') }
        if ($section.GetAttribute('locked') -eq 'true') { throw 'Unlock this section in OneNote desktop before creating a page.' }
        $attempted = $true
        $app.CreateNewPage($target, [ref]$pageId, 1)
        $created = $true
    } else { $pageId = $target }
    [string]$beforeXml = ''
    $app.GetPageContent($pageId, [ref]$beforeXml, 0, 2)
    $before = Parse-Page $beforeXml $pageId
    $modified = $before.DocumentElement.GetAttribute('lastModifiedTime')
    if ([string]::IsNullOrWhiteSpace($modified)) { throw 'OneNote returned no modification timestamp; refusing an unchecked update.' }
    $expected = [Xml.XmlConvert]::ToDateTime($modified, [Xml.XmlDateTimeSerializationMode]::Utc)
    $oldIds = @($before.DocumentElement.ChildNodes | Where-Object { $_ -is [Xml.XmlElement] -and $_.HasAttribute('objectID') } | ForEach-Object { $_.GetAttribute('objectID') })
    $bottom = 100.0
    foreach ($node in $before.DocumentElement.ChildNodes) {
        if ($node -isnot [Xml.XmlElement]) { continue }
        $pos = @(Nodes $node 'Position')
        $size = @(Nodes $node 'Size')
        if ($pos.Count -gt 0 -and $size.Count -gt 0) {
            $y = [double]::Parse($pos[0].GetAttribute('y'), [Globalization.CultureInfo]::InvariantCulture)
            $height = [double]::Parse($size[0].GetAttribute('height'), [Globalization.CultureInfo]::InvariantCulture)
            if ([double]::IsNaN($y + $height) -or [double]::IsInfinity($y + $height)) { throw 'Invalid page geometry.' }
            $bottom = [Math]::Max($bottom, $y + $height + 24)
        }
    }
    $changes = New-Object Xml.XmlDocument
    $page = $changes.CreateElement('one', 'Page', $ns)
    $page.SetAttribute('ID', $pageId)
    [void]$changes.AppendChild($page)
    if ($created) {
        $title = $changes.CreateElement('one', 'Title', $ns)
        Text-Node $changes $title ([string]$request.title)
        [void]$page.AppendChild($title)
    }
    # New outline only: never serialize existing objects back into UpdatePageContent.
    $outline = $changes.CreateElement('one', 'Outline', $ns)
    $position = $changes.CreateElement('one', 'Position', $ns)
    $position.SetAttribute('x', '36')
    $position.SetAttribute('y', $bottom.ToString([Globalization.CultureInfo]::InvariantCulture))
    $position.SetAttribute('z', '0')
    [void]$outline.AppendChild($position)
    $children = $changes.CreateElement('one', 'OEChildren', $ns)
    Text-Node $changes $children $text
    [void]$outline.AppendChild($children)
    [void]$page.AppendChild($outline)
    $attempted = $true
    $app.UpdatePageContent($changes.OuterXml, $expected, 2, $false)
    [string]$xml = ''
    $app.GetPageContent($pageId, [ref]$xml, 0, 2)
    $after = Parse-Page $xml $pageId
    $afterIds = @($after.DocumentElement.ChildNodes | Where-Object { $_ -is [Xml.XmlElement] -and $_.HasAttribute('objectID') } | ForEach-Object { $_.GetAttribute('objectID') })
    foreach ($id in $oldIds) { if ($id -cnotin $afterIds) { throw 'An existing page object is absent in the read-back.' } }
    $matches = @(Nodes $after.DocumentElement 'Outline' | Where-Object { $_.GetAttribute('objectID') -cnotin $oldIds -and (Plain $_) -ceq (Normalize $text) })
    if ($matches.Count -ne 1) { throw 'The new content block could not be verified in the read-back.' }
    if ($created -and (Plain (@(Nodes $after.DocumentElement 'Title')[0])) -cne (Normalize ([string]$request.title))) { throw 'The page title could not be verified.' }
    $result = @{xml=$xml;id=$pageId;operation=$request.operation;verified=$true}
} catch {
    $failed = $true
    $exception = $_.Exception
    while ($null -ne $exception.InnerException) { $exception = $exception.InnerException }
    $code = '{0:X8}' -f $exception.HResult
    $kind = 'desktop_error'
    $message = $_.Exception.Message
    if ($exception -is [ArgumentException]) { $kind = 'invalid_input' }
    if ($code -eq '8004201B') { $message += ' Unlock this section in OneNote desktop and retry.' }
    if ($attempted) {
        $kind = 'write_uncertain'
        $message += " The write may have taken effect. Inspect OneNote before retrying. Page ID: $pageId"
    }
    if ($code -eq '80042010' -and -not $created) {
        $kind = 'conflict'
        $message = 'The page changed before append; no content was appended. Read the page and retry deliberately.'
    }
    $result = @{error=@{kind=$kind;message=$message;page_id=$pageId;operation=$request.operation}}
} finally {
    if ($null -ne $app -and [Runtime.InteropServices.Marshal]::IsComObject($app)) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app) }
}
$result | ConvertTo-Json -Depth 8 -Compress
if ($failed) { exit 1 }
