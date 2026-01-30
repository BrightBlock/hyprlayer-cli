# HyprLayer Installer for Windows
# Install script for hyprlayer CLI

$ErrorActionPreference = "Stop"

# Installation directories
$InstallDir = "$env:USERPROFILE\.hyprlayer"
$BinDir = "$InstallDir\bin"

# Repository info
$Repo = "BrightBlock/hyprlayer-cli"
$GitHubAPI = "https://api.github.com/repos/$Repo/releases/latest"

# Auth header for private repos
# Try GITHUB_TOKEN env var first, then gh CLI
$Token = $env:GITHUB_TOKEN
if (-not $Token) {
    try {
        $Token = (gh auth token 2>$null)
    } catch {
        # gh CLI not available or not authenticated
    }
}

$Headers = @{}
if ($Token) {
    $Headers["Authorization"] = "token $Token"
}

Write-Host "Fetching latest release..." -ForegroundColor Cyan

try {
    $Release = Invoke-RestMethod -Uri $GitHubAPI -Headers $Headers
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

# Windows x86_64 only
$Binary = "hyprlayer-x86_64-pc-windows-msvc.exe"

Write-Host "Installing HyprLayer $Version..." -ForegroundColor Green

# Check for existing installation
if (Test-Path $InstallDir) {
    Write-Host "Warning: HyprLayer is already installed at $InstallDir" -ForegroundColor Yellow
    $Response = Read-Host "Do you want to reinstall? [y/N]"
    if ($Response -notmatch '^[Yy]$') {
        Write-Host "Installation cancelled."
        exit 0
    }
    Remove-Item -Recurse -Force $InstallDir
}

# Create installation directories
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null

# Download binary
Write-Host "Downloading $Binary ($Version)..." -ForegroundColor Cyan

if ($Token) {
    # Private repo: download via API
    $Asset = $Release.assets | Where-Object { $_.name -eq $Binary }
    if (-not $Asset) {
        Write-Host "Error: Could not find asset $Binary in release $Version" -ForegroundColor Red
        exit 1
    }
    $DownloadHeaders = $Headers.Clone()
    $DownloadHeaders["Accept"] = "application/octet-stream"
    Invoke-WebRequest -Uri $Asset.url -Headers $DownloadHeaders -OutFile "$BinDir\hyprlayer.exe"
} else {
    # Public repo: direct download
    $DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$Binary"
    Invoke-WebRequest -Uri $DownloadUrl -OutFile "$BinDir\hyprlayer.exe"
}

# Install Claude Code agents and commands
$ClaudeDest = "$env:USERPROFILE\.claude"
$ArchiveUrl = "https://github.com/$Repo/archive/refs/tags/$Version.zip"

Write-Host "Installing Claude Code agents and commands..." -ForegroundColor Cyan

$TempDir = New-Item -ItemType Directory -Force -Path "$env:TEMP\hyprlayer-install-$(Get-Random)"
try {
    $ZipPath = "$TempDir\repo.zip"
    if ($Token) {
        Invoke-WebRequest -Uri $ArchiveUrl -Headers $Headers -OutFile $ZipPath
    } else {
        Invoke-WebRequest -Uri $ArchiveUrl -OutFile $ZipPath
    }

    Expand-Archive -Path $ZipPath -DestinationPath $TempDir
    $ExtractedDir = Get-ChildItem -Path $TempDir -Directory | Where-Object { $_.Name -like "hyprlayer-cli-*" } | Select-Object -First 1

    if ($ExtractedDir -and (Test-Path "$($ExtractedDir.FullName)\claude")) {
        New-Item -ItemType Directory -Force -Path $ClaudeDest | Out-Null
        Copy-Item -Recurse -Force "$($ExtractedDir.FullName)\claude\*" $ClaudeDest
        Write-Host "Claude Code configuration installed to $ClaudeDest" -ForegroundColor Green
    } else {
        Write-Host "Warning: Could not find Claude Code configuration in release archive" -ForegroundColor Yellow
    }
} finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}

# Check for Visual C++ runtime
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
Write-Host ""
Write-Host "To use hyprlayer, add it to your PATH:" -ForegroundColor Yellow
Write-Host ""
Write-Host "  [Environment]::SetEnvironmentVariable('PATH', `$env:PATH + ';$BinDir', 'User')" -ForegroundColor Cyan
Write-Host ""
Write-Host "Or add $BinDir to your PATH manually via System Properties."
Write-Host ""
Write-Host "After updating PATH, restart your terminal and run:" -ForegroundColor Green
Write-Host "  hyprlayer --version"
Write-Host ""
Write-Host "To uninstall, simply remove: $InstallDir" -ForegroundColor Yellow
