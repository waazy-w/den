# Installs the latest den release on Windows. In PowerShell:
#   irm https://raw.githubusercontent.com/waazy-w/den/main/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$repo = 'waazy-w/den'
$dest = if ($env:DEN_INSTALL_DIR) { $env:DEN_INSTALL_DIR } else { Join-Path $HOME '.local\bin' }
# One x86_64 build; Windows on ARM runs it through its built-in emulation.
$asset = 'den-x86_64-windows.exe'
$url = "https://github.com/$repo/releases/latest/download/$asset"

New-Item -ItemType Directory -Force -Path $dest | Out-Null
$tmp = Join-Path $dest ".den-install-$PID.exe"
Write-Host "downloading $asset ..."
try {
    Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
} catch {
    Remove-Item -Force -ErrorAction SilentlyContinue $tmp
    Write-Error "couldn't download $asset from the latest release of $repo. If a new version was just published, try again in a few minutes."
}
$exe = Join-Path $dest 'den.exe'
# Clear out copies earlier updates moved aside (ones still running stay).
Get-ChildItem -Path $dest -Filter 'den.old*.exe' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
if (Test-Path $exe) {
    # A running den.exe can't be overwritten, but it can be moved aside.
    Move-Item -Force $exe (Join-Path $dest "den.old-$([DateTimeOffset]::Now.ToUnixTimeMilliseconds()).exe")
}
Move-Item -Force $tmp $exe
Write-Host "installed $(& $exe --version) to $exe"

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $dest)) {
    [Environment]::SetEnvironmentVariable('Path', "$dest;$userPath", 'User')
    $env:Path = "$dest;$env:Path"
    Write-Host "added $dest to your PATH (open a new terminal for other windows to see it)"
}
if (-not (Get-Command claude -ErrorAction SilentlyContinue)) {
    Write-Host "note: den drives Claude Code. Install it first: https://docs.claude.com/en/docs/claude-code"
}
Write-Host "run: den        (den alias makes typing claude open den too)"
