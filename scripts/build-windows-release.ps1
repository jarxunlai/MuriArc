<#
.SYNOPSIS
Builds an unsigned, production-mode MuriArc Windows desktop installer.

.DESCRIPTION
Runs from a clean Windows checkout at an exact commit, pins pnpm 11.5.0,
executes the release checks unless -SkipChecks is supplied, and publishes
MSI/NSIS artifacts plus SHA-256 evidence outside the repository.

No AI model is downloaded or invoked by this script.

.EXAMPLE
.\scripts\build-windows-release.ps1 `
  -ExpectedCommit (git rev-parse HEAD) `
  -RepoRoot (Get-Location).Path

.EXAMPLE
.\scripts\build-windows-release.ps1 `
  -ExpectedCommit (git rev-parse HEAD) `
  -RepoRoot (Get-Location).Path `
  -SkipChecks
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$ExpectedCommit,

    [Parameter(Mandatory = $true)]
    [string]$RepoRoot,

    [string]$BuildRoot,

    [switch]$SkipChecks
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if ([string]::IsNullOrWhiteSpace($BuildRoot)) {
    $BuildRoot = if (Test-Path -LiteralPath 'E:\Muriarc') {
        'E:\Muriarc\builds'
    } else {
        Join-Path $env:LOCALAPPDATA 'MuriArc\builds'
    }
}

function Assert-CommandSucceeded {
    param(
        [Parameter(Mandatory = $true)][string]$Step,
        [Parameter(Mandatory = $true)][bool]$Succeeded
    )
    if (-not $Succeeded) {
        throw "$Step failed"
    }
}

function Require-File {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required executable or file is missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

$GitExe = Require-File (Join-Path $env:ProgramFiles 'Git\cmd\git.exe')
$RustBin = Join-Path $env:USERPROFILE '.cargo\bin'
$CargoExe = Require-File (Join-Path $RustBin 'cargo.exe')
$RustcExe = Require-File (Join-Path $RustBin 'rustc.exe')
$RustupExe = Require-File (Join-Path $RustBin 'rustup.exe')
$NodeDir = Join-Path $env:ProgramFiles 'nodejs'
$NodeExe = Require-File (Join-Path $NodeDir 'node.exe')
$CorepackExe = Require-File (Join-Path $NodeDir 'corepack.cmd')
$PnpmExe = Require-File (Join-Path $NodeDir 'pnpm.CMD')
$CmdExe = Require-File (Join-Path $env:SystemRoot 'System32\cmd.exe')
$VsWhere = Require-File (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe')

$env:PATH = "$RustBin;$NodeDir;$env:SystemRoot\System32;$env:SystemRoot;$env:PATH"
$env:CI = 'true'
$VsRoot = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\2022\BuildTools'
if (-not (Test-Path -LiteralPath $VsRoot -PathType Container)) {
    $VsCandidates = @(& $VsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    Assert-CommandSucceeded 'Visual Studio discovery' $?
    $VsRoot = ($VsCandidates | Select-Object -First 1)
}
if (-not $VsRoot -or -not (Test-Path -LiteralPath $VsRoot -PathType Container)) {
    throw 'Visual Studio C++ Build Tools with the x64 MSVC toolchain is required.'
}
$VsDevCmd = Require-File (Join-Path $VsRoot 'Common7\Tools\VsDevCmd.bat')
$VsEnvironment = & $CmdExe /d /s /c "`"$VsDevCmd`" -no_logo -arch=x64 -host_arch=x64 && set"
Assert-CommandSucceeded 'Visual Studio developer environment initialization' $?
foreach ($Line in $VsEnvironment) {
    if ($Line -match '^([^=]+)=(.*)$') {
        Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2]
    }
}

$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
Set-Location -LiteralPath $RepoRoot
$env:GIT_CONFIG_COUNT = '1'
$env:GIT_CONFIG_KEY_0 = 'safe.directory'
$env:GIT_CONFIG_VALUE_0 = $RepoRoot

$ActualCommit = (& $GitExe rev-parse HEAD).Trim()
Assert-CommandSucceeded 'git rev-parse HEAD' $?
if ($ActualCommit -ne $ExpectedCommit) {
    throw "Commit mismatch: expected $ExpectedCommit, got $ActualCommit"
}
$Dirty = @(& $GitExe status --porcelain=v1 --untracked-files=all)
Assert-CommandSucceeded 'git status' $?
if ($Dirty.Count -ne 0) {
    $Dirty | ForEach-Object { Write-Error $_ }
    throw 'Release source tree is not clean.'
}

$ShortCommit = $ExpectedCommit.Substring(0, 12)
$RunId = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $ShortCommit
$EvidenceRoot = Join-Path $BuildRoot "desktop-evidence\$RunId"
$env:CARGO_TARGET_DIR = Join-Path $BuildRoot 'cargo-target\windows-release-shared'
New-Item -ItemType Directory -Force -Path $EvidenceRoot, $env:CARGO_TARGET_DIR | Out-Null

$TranscriptPath = Join-Path $EvidenceRoot 'build.log'
Start-Transcript -Path $TranscriptPath | Out-Null
try {
    Write-Output "expected_commit=$ExpectedCommit"
    Write-Output "actual_commit=$ActualCommit"
    Write-Output "repo_root=$RepoRoot"
    Write-Output "cargo_target=$env:CARGO_TARGET_DIR"
    Write-Output "evidence_root=$EvidenceRoot"
    Write-Output "powershell=$($PSVersionTable.PSVersion)"

    & $GitExe show --no-patch --format=fuller HEAD
    Assert-CommandSucceeded 'git show' $?
    & $RustcExe --version --verbose
    Assert-CommandSucceeded 'rustc --version' $?
    & $CargoExe --version --verbose
    Assert-CommandSucceeded 'cargo --version' $?
    & $RustupExe show active-toolchain
    Assert-CommandSucceeded 'rustup show active-toolchain' $?
    & $NodeExe --version
    Assert-CommandSucceeded 'node --version' $?
    & $CorepackExe --version
    Assert-CommandSucceeded 'corepack --version' $?

    & $CorepackExe prepare pnpm@11.5.0 --activate
    Assert-CommandSucceeded 'activate pnpm 11.5.0' $?
    $PnpmVersion = (& $PnpmExe --version).Trim()
    Assert-CommandSucceeded 'pnpm --version' $?
    if ($PnpmVersion -ne '11.5.0') {
        throw "pnpm 11.5.0 is required, got $PnpmVersion"
    }

    & $PnpmExe --dir ui install --frozen-lockfile
    Assert-CommandSucceeded 'pnpm install' $?

    if (-not $SkipChecks) {
        & $CargoExe fmt --all -- --check
        Assert-CommandSucceeded 'cargo fmt' $?
        & $CargoExe clippy --locked --workspace --all-targets --all-features -- -D warnings
        Assert-CommandSucceeded 'cargo clippy' $?
        & $CargoExe test --locked --workspace --all-targets --all-features
        Assert-CommandSucceeded 'cargo test' $?
        & $PnpmExe --dir ui audit --audit-level=high
        Assert-CommandSucceeded 'pnpm audit' $?
        & $PnpmExe --dir ui run test
        Assert-CommandSucceeded 'UI tests' $?
        & $PnpmExe --dir ui run typecheck
        Assert-CommandSucceeded 'UI typecheck' $?
        & $PnpmExe --dir ui exec playwright install chromium
        Assert-CommandSucceeded 'Playwright Chromium installation' $?
        & $PnpmExe --dir ui run test:e2e
        Assert-CommandSucceeded 'UI end-to-end tests' $?
        $env:VITE_MURIARC_GATEWAY = 'local'
        & $PnpmExe --dir ui run build
        Assert-CommandSucceeded 'local UI production build' $?
        Remove-Item Env:VITE_MURIARC_GATEWAY -ErrorAction SilentlyContinue
    }

    $TauriExe = Require-File (Join-Path $RepoRoot 'ui\node_modules\.bin\tauri.cmd')
    & $TauriExe build
    Assert-CommandSucceeded 'Tauri release bundle' $?

    $BundleRoot = Join-Path $env:CARGO_TARGET_DIR 'release\bundle'
    $Artifacts = @(Get-ChildItem -LiteralPath $BundleRoot -Recurse -File | Where-Object {
        $_.Extension -in '.msi', '.exe'
    })
    if ($Artifacts.Count -eq 0) {
        throw "No MSI or NSIS release artifact was produced under $BundleRoot"
    }

    $ArtifactRoot = Join-Path $BuildRoot "desktop-release\$RunId"
    New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null
    $Published = foreach ($Artifact in $Artifacts) {
        $Destination = Join-Path $ArtifactRoot $Artifact.Name
        Copy-Item -LiteralPath $Artifact.FullName -Destination $Destination -Force
        Get-Item -LiteralPath $Destination
    }

    $Published |
        Select-Object FullName, Length, LastWriteTimeUtc |
        Format-Table -AutoSize |
        Out-File -FilePath (Join-Path $EvidenceRoot 'bundle-files.txt') -Encoding utf8
    $Published |
        Get-FileHash -Algorithm SHA256 |
        Format-Table -AutoSize |
        Out-File -FilePath (Join-Path $EvidenceRoot 'bundle-sha256.txt') -Encoding utf8

    @"
expected_commit=$ExpectedCommit
actual_commit=$ActualCommit
repo_root=$RepoRoot
cargo_target=$env:CARGO_TARGET_DIR
bundle_root=$BundleRoot
artifact_root=$ArtifactRoot
evidence_root=$EvidenceRoot
checks_skipped=$($SkipChecks.IsPresent)
"@ | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'source-and-paths.txt') -Encoding utf8

    Write-Output 'MURIARC_DESKTOP_BUILD=PASS'
    Write-Output "artifact_root=$ArtifactRoot"
    $Published | ForEach-Object {
        $Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        Write-Output "artifact=$($_.FullName)"
        Write-Output "sha256=$Hash"
    }
}
finally {
    Stop-Transcript | Out-Null
}
