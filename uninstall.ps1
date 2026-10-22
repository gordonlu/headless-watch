[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'headless-watch'),
    [switch]$RemoveData,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$appRoot = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\', '/')
$driveRoot = [IO.Path]::GetPathRoot($appRoot).TrimEnd('\', '/')
if ($appRoot -eq $driveRoot -or $appRoot -eq [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\', '/') -or $appRoot -eq [IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd('\', '/')) { throw 'InstallDir must be a dedicated application directory.' }
if (-not (Test-Path -LiteralPath $appRoot)) { Write-Host 'No installation found.'; return }
if ((Get-Item -LiteralPath $appRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'InstallDir cannot be a junction or symbolic link.' }
$marker = Join-Path $appRoot 'install.json'
if (-not (Test-Path -LiteralPath $marker)) { throw 'Installation marker is missing; no files were removed.' }
$config = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
if ($config.application -ne 'headless-watch' -or $config.install_dir -ne $appRoot) { throw 'Installation marker does not match InstallDir.' }

# Resolve and inspect every removal target before making any mutations.
$names = @('bin', 'web', 'launch.ps1', 'uninstall.ps1')
if ($RemoveData) { $names += @('data', 'logs', 'install.json') }
$targets = @()
foreach ($name in $names) {
    $target = [IO.Path]::GetFullPath((Join-Path $appRoot $name))
    if (-not $target.StartsWith($appRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw "Unsafe removal target: $target" }
    if (Test-Path -LiteralPath $target) {
        $entries = @((Get-Item -LiteralPath $target -Force))
        if ((Get-Item -LiteralPath $target).PSIsContainer) { $entries += @(Get-ChildItem -LiteralPath $target -Force -Recurse) }
        if (@($entries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count -gt 0) { throw "Refusing linked removal target: $target" }
        $targets += $target
    }
}
$startupDir = [Environment]::GetFolderPath('Startup')
$shortcutPath = Join-Path $startupDir 'Headless Watch.lnk'
$launchArguments = '-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + (Join-Path $appRoot 'launch.ps1') + '"'
$ownsShortcut = $false
if (Test-Path -LiteralPath $shortcutPath) {
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    $ownsShortcut = $shortcut.Arguments -eq $launchArguments
}
Write-Host "Uninstall: $appRoot; remove data: $RemoveData; remove owned login shortcut: $ownsShortcut"
if ($DryRun) { Write-Host 'Dry run: no processes, files, or startup entries were changed.'; return }
$binary = Join-Path $appRoot 'bin/headless-watch.exe'
if (Test-Path -LiteralPath $binary) {
    & $binary stop
    if ($LASTEXITCODE -ne 0) { throw 'Could not stop the installed monitor; files were kept.' }
}
if ($ownsShortcut) { Remove-Item -LiteralPath $shortcutPath -Force }
foreach ($target in $targets) { Remove-Item -LiteralPath $target -Recurse -Force }
if ($RemoveData -and @(Get-ChildItem -LiteralPath $appRoot -Force).Count -eq 0) { Remove-Item -LiteralPath $appRoot -Force }
if ($RemoveData) { Write-Host 'Removed application files, local data, and logs.' } else { Write-Host "Removed application files. Data and logs remain in $appRoot." }
