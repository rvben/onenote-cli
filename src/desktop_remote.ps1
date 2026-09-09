# Windows OpenSSH runs outside the desktop session. A short-lived task runs as
# the same signed-in user, at limited privilege. No passwords are stored.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$task = 'OneNoteCli-' + [Guid]::NewGuid().ToString('N')
$directory = $null
$registered = $false
$started = $false
$envelope = $null
try {
    $envelope = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $user = [Security.Principal.WindowsIdentity]::GetCurrent()
    $directory = Join-Path $env:LOCALAPPDATA ('OneNoteCli\requests\' + $task)
    # Set permissions at creation, before any page text is written.
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @($user.User, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    [void][IO.Directory]::CreateDirectory($directory, $acl)
    $utf8 = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllBytes((Join-Path $directory 'bridge.ps1'), [Convert]::FromBase64String($envelope.script_base64))
    [IO.File]::WriteAllText((Join-Path $directory 'input.json'), ($envelope.request | ConvertTo-Json -Depth 10 -Compress), $utf8)
    # The runner has its own deadline, even if the SSH client disconnects.
    $runner = @'
$ErrorActionPreference = 'Stop'
$process = $null
try {
    $process = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoLogo -NoProfile -NonInteractive -STA -ExecutionPolicy Bypass -File bridge.ps1' -WorkingDirectory $pwd.Path -RedirectStandardInput input.json -RedirectStandardOutput output.json -RedirectStandardError error.txt -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(45000)) {
        $process.Kill()
        $process.WaitForExit()
        throw 'OneNote did not respond within 45 seconds; check its desktop for prompts.'
    }
    [IO.File]::WriteAllText((Join-Path $pwd.Path 'done'), 'complete')
} catch {
    [IO.File]::WriteAllText((Join-Path $pwd.Path 'failed'), $_.Exception.Message)
} finally {
    if ($null -ne $process) { $process.Dispose() }
}
'@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($runner))
    $action = New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument "-NoLogo -NoProfile -NonInteractive -STA -WindowStyle Hidden -EncodedCommand $encoded" -WorkingDirectory $directory
    $principal = New-ScheduledTaskPrincipal -UserId $user.Name -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds 60) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Settings $settings | Out-Null
    $registered = $true
    # Once Start is attempted, an interrupted write must be treated as uncertain.
    $started = $true
    Start-ScheduledTask -TaskName $task
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path (Join-Path $directory 'done'))) {
        if (Test-Path (Join-Path $directory 'failed')) { throw [IO.File]::ReadAllText((Join-Path $directory 'failed')) }
        if ($clock.Elapsed.TotalSeconds -gt 65) { throw 'Desktop task timed out. Sign in to Windows as the SSH user and open OneNote, then run onenote doctor.' }
        Start-Sleep -Milliseconds 200
    }
    $output = Join-Path $directory 'output.json'
    if ((Get-Item $output).Length -gt 33554432) { throw 'OneNote response exceeded 32 MiB; narrow the scope.' }
    $response = [IO.File]::ReadAllText($output)
    if ([string]::IsNullOrWhiteSpace($response)) { throw 'The desktop bridge returned no response; check OneNote in the signed-in desktop.' }
    [Console]::Write($response)
} catch {
    $kind = if ($started -and $envelope.write) { 'write_uncertain' } else { 'desktop_error' }
    $message = $_.Exception.Message
    if (-not $started) { $message += ' SSH desktop access requires a signed-in Windows user with permission to register an interactive scheduled task.' }
    if ($kind -eq 'write_uncertain') { $message += ' The write may have taken effect; inspect OneNote before retrying.' }
    [Console]::WriteLine((@{error=@{kind=$kind;message=$message}} | ConvertTo-Json -Compress))
} finally {
    if ($registered) {
        Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
    }
    if ($null -ne $directory -and (Test-Path $directory)) { Remove-Item -LiteralPath $directory -Recurse -Force -ErrorAction SilentlyContinue }
}
