#Requires -Version 5.1
<#
.SYNOPSIS
  Install the pinned Lean toolchain, build the formal project and audit theorems.
.DESCRIPTION
  Run from a Windows PowerShell terminal. The only automated installation is
  elan via the official winget package. Toolchain versions are pinned by
  formal/lean-toolchain. No GitHub Actions or permanent PATH edits.
  Any installation or proof failure exits nonzero and is logged.
#>
[CmdletBinding()]
param(
    [switch]$SkipInstall,
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$formal = Join-Path $root 'formal'
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $root 'target\lean-setup' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$log = Join-Path $OutputDirectory 'setup.log'
$statusPath = Join-Path $OutputDirectory 'status.json'
$status = [ordered]@{
    schema_version = 1
    kind = 'lean-local-setup'
    state = 'STARTED'
    expected_toolchain = ''
    steps = @()
}
function Save-State {
    $status | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $statusPath -Encoding UTF8
}
function Log([string]$message) {
    $line = ('[{0}] {1}' -f (Get-Date -Format o), $message)
    Write-Host $line
    Add-Content -LiteralPath $log -Value $line -Encoding UTF8
}
function Run-Step([string]$name, [string]$binary, [string[]]$arguments, [string]$workingDirectory) {
    Log ("START " + $name)
    Push-Location $workingDirectory
    try {
        & $binary @arguments 2>&1 | ForEach-Object { Log ([string]$_) }
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 1 }
        $status.steps += @{ name = $name; exit_code = $code }
        Save-State
        if ($code -ne 0) { throw "$name failed (exit $code)" }
    } finally { Pop-Location }
}
try {
    if (-not (Test-Path (Join-Path $formal 'lean-toolchain'))) {
        throw 'formal/lean-toolchain is missing'
    }
    $toolchain = (Get-Content (Join-Path $formal 'lean-toolchain') -Raw).Trim()
    if ($toolchain -ne 'leanprover/lean4:v4.24.0') {
        throw ("Unexpected pinned toolchain: " + $toolchain)
    }
    $status.expected_toolchain = $toolchain
    Save-State

    $elanBin = Join-Path $HOME '.elan\bin'
    $env:PATH = "$elanBin;$env:PATH"
    if (-not (Get-Command elan -ErrorAction SilentlyContinue)) {
        if ($SkipInstall) { throw 'elan is not installed and -SkipInstall was given' }
        if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
            throw 'winget is missing. Install Windows App Installer or elan from https://github.com/leanprover/elan'
        }
        Run-Step 'install-elan' 'winget' @('install','--exact','--id','Lean.Elan','--accept-package-agreements','--accept-source-agreements') $root
        $env:PATH = "$elanBin;$env:PATH"
    }
    if (-not (Get-Command elan -ErrorAction SilentlyContinue)) { throw 'elan still missing after installation' }
    Run-Step 'install-pinned-toolchain' 'elan' @('toolchain','install',$toolchain) $formal
    if (-not (Get-Command lake -ErrorAction SilentlyContinue)) { throw 'lake is missing after elan toolchain installation' }
    Run-Step 'lean-version' 'lake' @('env','lean','--version') $formal
    Run-Step 'lean-build' 'lake' @('build') $formal
    $python = Get-Command py -ErrorAction SilentlyContinue
    if ($python) {
        Run-Step 'kernel-audit' 'py' @('scripts/check_formal.py') $root
    } elseif (Get-Command python -ErrorAction SilentlyContinue) {
        Run-Step 'kernel-audit' 'python' @('scripts/check_formal.py') $root
    } else { throw 'Python 3 is missing; kernel audit cannot run' }
    $status.state = 'PASSED'
    Save-State
    Log "SUCCESS. Evidence: $statusPath"
    exit 0
} catch {
    $status.state = 'FAILED'
    $status.reason = [string]$_
    Save-State
    Log ("FAIL: " + [string]$_)
    exit 1
}
