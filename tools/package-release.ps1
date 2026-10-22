[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Get-Content -LiteralPath (Join-Path $repoRoot 'backend/Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Could not read package version.' }
$architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$name = "headless-watch-$version-windows-$architecture"
$dist = Join-Path $repoRoot 'dist'
$stage = [IO.Path]::GetFullPath((Join-Path $dist $name))
if (-not $stage.StartsWith($dist + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe package stage.' }
if (-not $SkipBuild) {
    & pnpm --dir (Join-Path $repoRoot 'frontend') install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'Dashboard dependency installation failed.' }
    & pnpm --dir (Join-Path $repoRoot 'frontend') build
    if ($LASTEXITCODE -ne 0) { throw 'Dashboard build failed.' }
    & cargo build --release --manifest-path (Join-Path $repoRoot 'backend/Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
}
$binary = Join-Path $repoRoot 'backend/target/release/headless-watch.exe'
$web = Join-Path $repoRoot 'frontend/dist'
if (-not (Test-Path -LiteralPath $binary) -or -not (Test-Path -LiteralPath (Join-Path $web 'index.html'))) { throw 'Release artifacts missing; build before packaging.' }
if (Test-Path -LiteralPath $stage) {
    $entries = @((Get-Item -LiteralPath $stage -Force)) + @(Get-ChildItem -LiteralPath $stage -Force -Recurse)
    if (@($entries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count -gt 0) { throw 'Refusing linked package stage.' }
    Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory -Path (Join-Path $stage 'bin'), (Join-Path $stage 'web') -Force | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'bin/headless-watch.exe')
Copy-Item -Path (Join-Path $web '*') -Destination (Join-Path $stage 'web') -Recurse
Copy-Item -LiteralPath (Join-Path $repoRoot 'docs') -Destination (Join-Path $stage 'docs') -Recurse
foreach ($file in @('README.md', 'LICENSE', 'install.ps1', 'uninstall.ps1')) { Copy-Item -LiteralPath (Join-Path $repoRoot $file) -Destination (Join-Path $stage $file) }
$archive = Join-Path $dist "$name.zip"
Compress-Archive -LiteralPath $stage -DestinationPath $archive -Force
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$archive.sha256", "$hash  $name.zip`n", (New-Object Text.UTF8Encoding($false)))
Write-Host "Created $archive"
