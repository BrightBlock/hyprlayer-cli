# Verify hyprlayer is installed at >= the required version.
# Usage: powershell -File check-hyprlayer-version.ps1 <required_version>
# Example: powershell -File check-hyprlayer-version.ps1 1.5.2
# Exits 0 if OK; exits 1 with an install/upgrade hint otherwise.
#
# Cross-platform notes: PowerShell 5.1+ (ships with Windows 10/11) or PowerShell 7+.
# Cwd-independent. Detects the user's package manager by probing PATH; falls back
# to a generic install URL when none is recognized.

param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Required
)

function Test-Cmd($name) {
    [bool](Get-Command $name -ErrorAction SilentlyContinue)
}

function Get-InstallHint {
    if     (Test-Cmd brew)   { return "brew tap brightblock/tap; brew install hyprlayer" }
    elseif (Test-Cmd scoop)  { return "scoop bucket add brightblock https://github.com/BrightBlock/scoop-bucket; scoop install hyprlayer" }
    elseif (Test-Cmd winget) { return "winget install BrightBlock.Hyprlayer" }
    elseif (Test-Cmd yay)    { return "yay -S hyprlayer-bin" }
    elseif (Test-Cmd paru)   { return "paru -S hyprlayer-bin" }
    else                     { return "see https://github.com/BrightBlock/hyprlayer-cli#install" }
}

function Get-UpgradeHint {
    if     (Test-Cmd brew)   { return "brew upgrade hyprlayer" }
    elseif (Test-Cmd scoop)  { return "scoop update hyprlayer" }
    elseif (Test-Cmd winget) { return "winget upgrade BrightBlock.Hyprlayer" }
    elseif (Test-Cmd yay)    { return "yay -Syu hyprlayer-bin" }
    elseif (Test-Cmd paru)   { return "paru -Syu hyprlayer-bin" }
    else                     { return "see https://github.com/BrightBlock/hyprlayer-cli#install" }
}

function Compare-SemVer($have, $want) {
    $a = ($have -split '\.') + @('0','0','0') | Select-Object -First 3
    $b = ($want -split '\.') + @('0','0','0') | Select-Object -First 3
    for ($i = 0; $i -lt 3; $i++) {
        $av = [int]$a[$i]; $bv = [int]$b[$i]
        if ($av -lt $bv) { return -1 }
        if ($av -gt $bv) { return 1 }
    }
    return 0
}

$verLine = $null
try { $verLine = (& hyprlayer --version 2>$null) } catch { $verLine = $null }

if (-not $verLine) {
    Write-Output ("hyprlayer not found. Install: " + (Get-InstallHint))
    exit 1
}

$have = ($verLine -split '\s+')[1] -replace '\(.*$', '' -replace '\s', ''

if ((Compare-SemVer $have $Required) -lt 0) {
    Write-Output ("hyprlayer >= $Required required (have $have). Upgrade: " + (Get-UpgradeHint))
    exit 1
}

exit 0
