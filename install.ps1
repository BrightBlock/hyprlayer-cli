$ErrorActionPreference = "Stop"

$InstallDir = "$env:USERPROFILE\.hyprlayer"
$BinDir = "$InstallDir\bin"

$Repo = "BrightBlock/hyprlayer-cli"
$GitHubAPI = "https://api.github.com/repos/$Repo/releases/latest"

Write-Host "Fetching latest release..." -ForegroundColor Cyan

try {
    $Release = Invoke-RestMethod -Uri $GitHubAPI
} catch {
    Write-Host "Error: Could not fetch release information" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    exit 1
}

$Version = $Release.tag_name
if (-not $Version) {
    Write-Host "Error: Could not determine latest release version" -ForegroundColor Red
    exit 1
}

$Binary = "hyprlayer-x86_64-pc-windows-msvc.exe"

Write-Host "Installing HyprLayer $Version..." -ForegroundColor Green

if (Test-Path $InstallDir) {
    Write-Host "Warning: HyprLayer is already installed at $InstallDir" -ForegroundColor Yellow
    $Response = Read-Host "Do you want to reinstall? [y/N]"
    if ($Response -notmatch '^[Yy]$') {
        Write-Host "Installation cancelled."
        exit 0
    }
}

$Asset = $Release.assets | Where-Object { $_.name -eq $Binary } | Select-Object -First 1
if (-not $Asset) {
    Write-Host "Error: $Binary not found in release $Version assets" -ForegroundColor Red
    exit 1
}
if (-not $Asset.digest -or -not ($Asset.digest -match '^sha256:[A-Fa-f0-9]{64}$')) {
    Write-Host "Error: GitHub release $Version exposes no valid sha256 digest for $Binary" -ForegroundColor Red
    Write-Host "       Refusing to install an unverified binary." -ForegroundColor Red
    exit 1
}
$Expected = $Asset.digest.Substring(7).ToLower()

Write-Host "Downloading $Binary ($Version)..." -ForegroundColor Cyan

$DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$Binary"
$BinaryPath = "$BinDir\hyprlayer.exe"
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("hyprlayer-install-" + [guid]::NewGuid().ToString("N"))
$TempBinaryPath = Join-Path $TempDir "hyprlayer.exe"

New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempBinaryPath

    $Actual = (Get-FileHash -Algorithm SHA256 $TempBinaryPath).Hash.ToLower()
    if ($Actual -ne $Expected) {
        Write-Host "Error: SHA256 mismatch for $Binary" -ForegroundColor Red
        Write-Host "  expected: $Expected" -ForegroundColor Red
        Write-Host "  actual:   $Actual" -ForegroundColor Red
        exit 1
    }
    Write-Host "Checksum verified ($Expected)" -ForegroundColor Green

    if (Test-Path $InstallDir) {
        Remove-Item -Recurse -Force $InstallDir
    }
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Move-Item -Force $TempBinaryPath $BinaryPath
    Set-Content -Path "$BinDir\hyprlayer.install-method" -Value "windows-installer" -NoNewline -Encoding ascii
}
finally {
    if (Test-Path $TempDir) {
        Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
    }
}

Write-Host ""
Write-Host "Agent files will be installed when you run 'hyprlayer thoughts init'" -ForegroundColor Yellow
Write-Host "You'll be prompted to choose between Claude Code and GitHub Copilot."

$VCRuntimeInstalled = Test-Path "$env:SystemRoot\System32\vcruntime140.dll"
if (-not $VCRuntimeInstalled) {
    Write-Host ""
    Write-Host "Warning: Visual C++ Runtime not found!" -ForegroundColor Red
    Write-Host "HyprLayer requires the Visual C++ Redistributable to run." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Download and install from:" -ForegroundColor Yellow
    Write-Host "  https://aka.ms/vs/17/release/vc_redist.x64.exe" -ForegroundColor Cyan
    Write-Host ""
}

Write-Host ""
Write-Host "Installation successful!" -ForegroundColor Green
Write-Host ""
Write-Host "HyprLayer has been installed to: $BinDir"

$UserPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($UserPath -notlike "*$BinDir*") {
    [Environment]::SetEnvironmentVariable('PATH', "$UserPath;$BinDir", 'User')
    $env:PATH = "$env:PATH;$BinDir"
    Write-Host ""
    Write-Host "Added $BinDir to your PATH." -ForegroundColor Green
    Write-Host "Restart your terminal for PATH changes to take effect." -ForegroundColor Yellow
} else {
    Write-Host "$BinDir is already in your PATH." -ForegroundColor Green
}

Write-Host ""
Write-Host "Run 'hyprlayer --version' to verify the installation." -ForegroundColor Green
Write-Host ""
Write-Host "To uninstall, simply remove: $InstallDir" -ForegroundColor Yellow
