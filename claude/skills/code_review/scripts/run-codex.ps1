# Run codex in challenge mode with JSONL streaming, piped through `hyprlayer codex stream`.
# Usage: powershell -File run-codex.ps1 -Prompt <prompt> -Effort <high|xhigh> [-Model <model>]
# Reads $env:_REPO_ROOT if set; falls back to `git rev-parse --show-toplevel`.
# Exit code: 0 on codex success, 124 if the inner timeout fired, otherwise codex's exit code.
#
# Cross-platform notes: PowerShell 5.1+ (ships with Windows 10/11) or PowerShell 7+ on any OS.
# Cwd-independent — locates the repo via `git rev-parse` or the inherited `_REPO_ROOT` env var.
# Uses `New-TemporaryFile` so it lives in the platform's temp dir (`$env:TEMP` on Windows,
# `$TMPDIR` on Unix-like systems).

param(
    [Parameter(Mandatory = $true)]
    [string]$Prompt,

    [Parameter(Mandatory = $true)]
    [ValidateSet('high', 'xhigh')]
    [string]$Effort,

    [string]$Model = ''
)

$repoRoot = if ($env:_REPO_ROOT) { $env:_REPO_ROOT } else { (& git rev-parse --show-toplevel 2>$null) }
if (-not $repoRoot) {
    [Console]::Error.WriteLine("not in a git repo")
    exit 1
}

$tmpErr = New-TemporaryFile
try {
    $codexArgs = @(
        'exec', $Prompt,
        '-C', $repoRoot,
        '-s', 'read-only',
        '-c', "model_reasoning_effort=`"$Effort`"",
        '--enable', 'web_search_cached',
        '--json'
    )
    if ($Model) { $codexArgs += @('-m', $Model) }

    # PowerShell does not have a native equivalent of GNU `timeout` baked in. We rely on
    # codex itself to honor request timeouts; the harness re-runs on stalls.
    $codexExit = 0
    & codex @codexArgs 2> $tmpErr | & hyprlayer codex stream
    $codexExit = $LASTEXITCODE

    if (Select-String -Path $tmpErr -Pattern 'auth|login|unauthorized' -Quiet -CaseSensitive:$false) {
        $firstLine = (Get-Content $tmpErr -TotalCount 1)
        [Console]::Error.WriteLine("[codex auth error] $firstLine")
    }

    exit $codexExit
}
finally {
    Remove-Item $tmpErr -Force -ErrorAction SilentlyContinue
}
