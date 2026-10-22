[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'headless-watch'),
    [ValidateRange(1, 65535)][int]$Port = 8765,
    [switch]$SkipStart,
    [switch]$NoAutoStart,
    [switch]$SkipBuild,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$sourceRoot = $PSScriptRoot
$appRoot = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\', '/')
$driveRoot = [IO.Path]::GetPathRoot($appRoot).TrimEnd('\', '/')
if ($appRoot -eq $driveRoot -or $appRoot -eq [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\', '/') -or $appRoot -eq [IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd('\', '/') -or $appRoot -eq [IO.Path]::GetFullPath($sourceRoot).TrimEnd('\', '/')) {
    throw 'InstallDir must be a dedicated application directory, not a drive, profile, LocalAppData, or source root.'
}
$configPath = Join-Path $appRoot 'install.json'
if (Test-Path -LiteralPath $appRoot) {
    if ((Get-Item -LiteralPath $appRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'InstallDir cannot be a junction or symbolic link.' }
    $items = @(Get-ChildItem -LiteralPath $appRoot -Force)
    if ($items.Count -gt 0) {
        if (Test-Path -LiteralPath $configPath) {
            $previous = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
            if ($previous.application -ne 'headless-watch' -or $previous.install_dir -ne $appRoot) { throw 'Installation marker does not match InstallDir.' }
        } else {
            # A source run creates only data/ before the first installation.
            $unrecognized = @($items | Where-Object { $_.Name -notin @('data', 'logs') -or -not $_.PSIsContainer -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) })
            if ($unrecognized.Count -gt 0) { throw 'The nonempty InstallDir contains unrelated files and has no installation marker.' }
        }
    }
}
$startupDir = [Environment]::GetFolderPath('Startup')
$shortcutPath = Join-Path $startupDir 'Headless Watch.lnk'
$launchScript = Join-Path $appRoot 'launch.ps1'
$launchArguments = '-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $launchScript + '"'
Write-Host "Install: $appRoot"
Write-Host "Data: $(Join-Path $appRoot 'data')"
Write-Host "Dashboard: http://127.0.0.1:$Port"
Write-Host "Login startup: $(-not $NoAutoStart); start now: $(-not $SkipStart)"
if ($DryRun) { Write-Host 'Dry run: no build, files, processes, or startup entries were changed.'; return }

# Only the current user's Startup folder is used; registration requires no admin rights.
if (-not $NoAutoStart -and (Test-Path -LiteralPath $shortcutPath)) {
    $existing = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    if ($existing.Arguments -ne $launchArguments) { throw 'A different installation owns the Headless Watch startup shortcut. Remove that installation first or use -NoAutoStart.' }
}
$prebuilt = (Test-Path -LiteralPath (Join-Path $sourceRoot 'bin/headless-watch.exe')) -and (Test-Path -LiteralPath (Join-Path $sourceRoot 'web/index.html'))
if ($prebuilt) {
    $binarySource = Join-Path $sourceRoot 'bin/headless-watch.exe'
    $webSource = Join-Path $sourceRoot 'web'
} else {
    if (-not $SkipBuild) {
        & pnpm --dir (Join-Path $sourceRoot 'frontend') install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) { throw 'Dashboard dependency installation failed.' }
        & pnpm --dir (Join-Path $sourceRoot 'frontend') build
        if ($LASTEXITCODE -ne 0) { throw 'Dashboard build failed.' }
        & cargo build --release --manifest-path (Join-Path $sourceRoot 'backend/Cargo.toml')
        if ($LASTEXITCODE -ne 0) { throw 'Rust build failed. Install Rust and the Visual Studio C++ Build Tools.' }
    }
    $binarySource = Join-Path $sourceRoot 'backend/target/release/headless-watch.exe'
    $webSource = Join-Path $sourceRoot 'frontend/dist'
    if (-not (Test-Path -LiteralPath $binarySource) -or -not (Test-Path -LiteralPath (Join-Path $webSource 'index.html'))) { throw 'Build artifacts are missing; rerun without -SkipBuild.' }
}

# Stop only the old, identity-verified monitor before replacing a locked executable.
$installedBinary = Join-Path $appRoot 'bin/headless-watch.exe'
if (Test-Path -LiteralPath $installedBinary) {
    & $installedBinary stop
    if ($LASTEXITCODE -ne 0) { throw 'Could not stop the installed monitor; installation was not replaced.' }
}
foreach ($name in @('bin', 'web', 'data', 'logs')) {
    $directory = Join-Path $appRoot $name
    if ((Test-Path -LiteralPath $directory) -and ((Get-Item -LiteralPath $directory).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing reparse-point installation directory: $directory" }
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}
Copy-Item -LiteralPath $binarySource -Destination $installedBinary -Force
# Replace only regular files in our dedicated web directory; reject links before removal.
$webTarget = Join-Path $appRoot 'web'
$webEntries = @(Get-ChildItem -LiteralPath $webTarget -Force -Recurse)
if (@($webEntries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count -gt 0) { throw 'Refusing linked files in the installed web directory.' }
Get-ChildItem -LiteralPath $webTarget -Force | Remove-Item -Recurse -Force
Copy-Item -Path (Join-Path $webSource '*') -Destination $webTarget -Recurse -Force
Copy-Item -LiteralPath (Join-Path $sourceRoot 'uninstall.ps1') -Destination (Join-Path $appRoot 'uninstall.ps1') -Force
$configuration = [ordered]@{ application = 'headless-watch'; install_dir = $appRoot; port = $Port; data_dir = (Join-Path $appRoot 'data'); web_dir = $webTarget }
$utf8 = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText($configPath, ($configuration | ConvertTo-Json), $utf8)
$launcher = @'
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'bin/headless-watch.exe') start
exit $LASTEXITCODE
'@
[IO.File]::WriteAllText($launchScript, $launcher, $utf8)
if (-not $NoAutoStart) {
    New-Item -ItemType Directory -Path $startupDir -Force | Out-Null
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    $shortcut.TargetPath = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $shortcut.Arguments = $launchArguments
    $shortcut.WorkingDirectory = $appRoot
    $shortcut.Description = 'Headless Watch current-user background monitor'
    $shortcut.WindowStyle = 7
    $shortcut.Save()
}
if (-not $SkipStart) {
    & $installedBinary start
    if ($LASTEXITCODE -ne 0) { throw 'Installed, but startup failed. Inspect logs/headless-watch.log in InstallDir.' }
}
Write-Host "Installed. Use & '$installedBinary' start|status|stop|open."
Write-Host "Uninstall: & '$(Join-Path $appRoot 'uninstall.ps1')' (data is kept by default)."
