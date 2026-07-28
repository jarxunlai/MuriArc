<#
.SYNOPSIS
Builds a production-mode MuriArc Windows desktop installer and signed updater artifacts.

.DESCRIPTION
Runs from a clean Windows checkout at the exact freshly fetched canonical
origin/main commit, pins pnpm 11.5.0, always executes every release check,
and publishes MSI/NSIS artifacts plus SHA-256 evidence outside the repository.

No AI model is downloaded or invoked by this script.
The updater Minisign private key is read only from TAURI_SIGNING_PRIVATE_KEY;
the matching public key is read from MURIARC_DESKTOP_UPDATER_PUBLIC_KEY.

.EXAMPLE
.\scripts\build-windows-release.ps1 `
  -ExpectedCommit (git rev-parse HEAD) `
  -RepoRoot (Get-Location).Path

#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$ExpectedCommit,

    [Parameter(Mandatory = $true)]
    [string]$RepoRoot,

    [string]$BuildRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$env:PATHEXT = '.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC;.CPL'

if ([string]::IsNullOrWhiteSpace($env:MURIARC_DESKTOP_UPDATER_PUBLIC_KEY)) {
    throw 'MURIARC_DESKTOP_UPDATER_PUBLIC_KEY is required for a release build.'
}
if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
    throw 'TAURI_SIGNING_PRIVATE_KEY is required to produce signed updater artifacts.'
}
if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD)) {
    throw 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD is required for a release build.'
}

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

$OriginUrl = (& $GitExe remote get-url origin).Trim().TrimEnd('/')
Assert-CommandSucceeded 'git origin URL' $?
if ($OriginUrl -notin @('https://github.com/jarxunlai/MuriArc.git', 'https://github.com/jarxunlai/MuriArc')) {
    throw "Release source origin is not the canonical GitHub repository: $OriginUrl"
}
& $GitExe fetch --no-tags --prune origin '+refs/heads/main:refs/remotes/origin/main'
Assert-CommandSucceeded 'fetch canonical origin/main' $?

$ActualCommit = (& $GitExe rev-parse HEAD).Trim()
Assert-CommandSucceeded 'git rev-parse HEAD' $?
$OriginMain = (& $GitExe rev-parse refs/remotes/origin/main).Trim()
Assert-CommandSucceeded 'git origin/main identity' $?
if ($ActualCommit -ne $ExpectedCommit) {
    throw "Commit mismatch: expected $ExpectedCommit, got $ActualCommit"
}
if ($OriginMain -ne $ExpectedCommit) {
    throw "Release source must equal the freshly fetched origin/main tip: expected $ExpectedCommit, got $OriginMain"
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

$AllowedSensitiveEnvironment = @(
    'MURIARC_DESKTOP_UPDATER_PUBLIC_KEY',
    'TAURI_SIGNING_PRIVATE_KEY',
    'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'
)
$SensitiveEnvironment = @(Get-ChildItem Env: | Where-Object {
    ($_.Name -like 'VITE_*' -or
     $_.Name -match '(?i)(?:^|_)(?:api_?key|credential|csrf|master_?key|password|passwd|private_?key|secret|session|token)(?:$|_)') -and
    $_.Name -notin $AllowedSensitiveEnvironment
})
$SanitizedEnvironmentVariableCount = $SensitiveEnvironment.Count
$SensitiveEnvironment | ForEach-Object {
    Remove-Item -LiteralPath "Env:$($_.Name)" -ErrorAction SilentlyContinue
}

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

    $TauriExe = Require-File (Join-Path $RepoRoot 'ui\node_modules\.bin\tauri.cmd')
    & $TauriExe build
    Assert-CommandSucceeded 'Tauri release bundle' $?

    $BundleRoot = Join-Path $env:CARGO_TARGET_DIR 'release\bundle'
    $Installers = @(Get-ChildItem -LiteralPath $BundleRoot -Recurse -File | Where-Object {
        $_.Extension -in '.msi', '.exe'
    })
    if ($Installers.Count -eq 0) {
        throw "No MSI or NSIS release artifact was produced under $BundleRoot"
    }
    $UpdaterSignatures = @(Get-ChildItem -LiteralPath $BundleRoot -Recurse -File | Where-Object {
        $_.Extension -eq '.sig'
    })
    if ($UpdaterSignatures.Count -eq 0) {
        throw "No signed updater artifact was produced under $BundleRoot"
    }
    $UpdaterArchives = @(Get-ChildItem -LiteralPath $BundleRoot -Recurse -File | Where-Object {
        $_.Extension -in '.zip', '.gz'
    })
    if ($UpdaterArchives.Count -eq 0) {
        throw "No updater archive was produced under $BundleRoot"
    }
    $Artifacts = @($Installers + $UpdaterSignatures + @(
        Get-ChildItem -LiteralPath $BundleRoot -Recurse -File | Where-Object {
            $_.Extension -in '.zip', '.gz' -and $_.Name -notin $UpdaterSignatures.Name
        }
    ) | Sort-Object FullName -Unique)

    $ArtifactRoot = Join-Path $BuildRoot "desktop-release\$RunId"
    if (Test-Path -LiteralPath $ArtifactRoot) {
        throw "Desktop release artifact root already exists: $ArtifactRoot"
    }
    New-Item -ItemType Directory -Path $ArtifactRoot | Out-Null
    $Published = foreach ($Artifact in $Artifacts) {
        $Destination = Join-Path $ArtifactRoot $Artifact.Name
        if (Test-Path -LiteralPath $Destination) {
            throw "Duplicate Windows release artifact name: $($Artifact.Name)"
        }
        Copy-Item -LiteralPath $Artifact.FullName -Destination $Destination
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
origin_main=$OriginMain
origin_url=$OriginUrl
repo_root=$RepoRoot
cargo_target=$env:CARGO_TARGET_DIR
bundle_root=$BundleRoot
artifact_root=$ArtifactRoot
evidence_root=$EvidenceRoot
checks_skipped=False
"@ | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'source-and-paths.txt') -Encoding utf8

    $PostBuildDirty = @(& $GitExe status --porcelain=v1 --untracked-files=all)
    Assert-CommandSucceeded 'post-build git status' $?
    if ($PostBuildDirty.Count -ne 0) {
        $PostBuildDirty | ForEach-Object { Write-Error $_ }
        throw 'Windows release build dirtied the source tree.'
    }
    @"
checks_skipped=False
clean_tree_before_and_after=True
sanitized_environment_variable_count=$SanitizedEnvironmentVariableCount
"@ | Add-Content -LiteralPath (Join-Path $EvidenceRoot 'source-and-paths.txt') -Encoding utf8

    Write-Output 'MURIARC_DESKTOP_BUILD=PASS'
    Write-Output "artifact_root=$ArtifactRoot"
    $Published | ForEach-Object {
        $Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        Write-Output "artifact=$($_.FullName)"
        Write-Output "sha256=$Hash"
    }
}
finally {
    Remove-Item Env:VITE_MURIARC_GATEWAY -ErrorAction SilentlyContinue
    Stop-Transcript | Out-Null
}
