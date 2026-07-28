<#
.SYNOPSIS
Builds an unsigned MuriArc Windows Tester ZIP with synthetic standard-v1 data.

.DESCRIPTION
This script is deliberately separate from the signed v1.0.0 release pipeline.
It accepts only an exact, clean origin/main checkout, builds an isolated debug
Desktop binary, asks that exact binary to seed and verify a fresh E0001 SQLite
data root, performs an isolated startup smoke test, scans the package for
credential-like material, and emits a verified ZIP plus SHA-256 evidence.

The resulting package is unsigned, contains synthetic data, and is not for
production. It must be published only under a tester-specific prerelease tag.

.EXAMPLE
.\scripts\build-windows-tester.ps1 `
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

function Require-File {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required file is missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Require-Directory {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "Required directory is missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function ConvertTo-NativeArgument {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)
    if ($Value.Length -gt 0 -and $Value -notmatch '[\s&|<>^"]') {
        return $Value
    }
    # Start-Process in Windows PowerShell 5.1 accepts one argument string.
    # Apply the CommandLineToArgvW/MSVC escaping rules so paths with spaces,
    # embedded quotes, or trailing backslashes survive that second parse.
    $Builder = New-Object System.Text.StringBuilder
    $Backslash = [char]92
    [void]$Builder.Append('"')
    $PendingBackslashes = 0
    foreach ($Character in $Value.ToCharArray()) {
        if ($Character -eq $Backslash) {
            $PendingBackslashes += 1
            continue
        }
        if ($Character -eq '"') {
            [void]$Builder.Append($Backslash, ($PendingBackslashes * 2) + 1)
            [void]$Builder.Append('"')
        } else {
            [void]$Builder.Append($Backslash, $PendingBackslashes)
            [void]$Builder.Append($Character)
        }
        $PendingBackslashes = 0
    }
    [void]$Builder.Append($Backslash, $PendingBackslashes * 2)
    [void]$Builder.Append('"')
    return $Builder.ToString()
}

function Invoke-NativeCapture {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Step,
        [switch]$AllowFailure
    )
    $Token = [Guid]::NewGuid().ToString('N')
    $StdoutPath = Join-Path $env:TEMP "$Token.stdout"
    $StderrPath = Join-Path $env:TEMP "$Token.stderr"
    try {
        $ArgumentLine = ($Arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' '
        $Process = Start-Process -FilePath $FilePath -ArgumentList $ArgumentLine `
            -WorkingDirectory (Get-Location).ProviderPath -NoNewWindow -Wait -PassThru `
            -RedirectStandardOutput $StdoutPath -RedirectStandardError $StderrPath
        $Stdout = ''
        if (Test-Path -LiteralPath $StdoutPath) {
            $Stdout = [System.IO.File]::ReadAllText($StdoutPath)
        }
        $Stderr = ''
        if (Test-Path -LiteralPath $StderrPath) {
            $Stderr = [System.IO.File]::ReadAllText($StderrPath)
        }
        if ($Process.ExitCode -ne 0 -and -not $AllowFailure) {
            if ($Stdout) { Write-Output $Stdout.TrimEnd() }
            if ($Stderr) { Write-Output $Stderr.TrimEnd() }
            throw "$Step failed with exit code $($Process.ExitCode)"
        }
        return [pscustomobject]@{
            ExitCode = $Process.ExitCode
            Stdout = $Stdout.Trim()
            Stderr = $Stderr.Trim()
        }
    }
    finally {
        Remove-Item -LiteralPath $StdoutPath, $StderrPath -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-Native {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Step
    )
    $Result = Invoke-NativeCapture -FilePath $FilePath -Arguments $Arguments -Step $Step
    if ($Result.Stdout) { Write-Output $Result.Stdout }
    if ($Result.Stderr) { Write-Output $Result.Stderr }
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value
    )
    $Encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Value, $Encoding)
}

function Write-Json {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][object]$Value
    )
    Write-Utf8NoBom -Path $Path -Value (($Value | ConvertTo-Json -Depth 12) + "`n")
}

function Get-RelativePackagePath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $RootFull = (Get-Item -LiteralPath $Root -Force).FullName.TrimEnd('\', '/')
    $PathFull = (Get-Item -LiteralPath $Path -Force).FullName
    $RootPrefix = $RootFull + [System.IO.Path]::DirectorySeparatorChar
    if (-not $PathFull.StartsWith($RootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Package path escapes its root: $PathFull"
    }
    $RootUri = New-Object System.Uri($RootPrefix)
    $PathUri = New-Object System.Uri($PathFull)
    return [System.Uri]::UnescapeDataString($RootUri.MakeRelativeUri($PathUri).ToString())
}

function Get-FileDigestMap {
    param([Parameter(Mandatory = $true)][string]$Root)
    $Map = [ordered]@{}
    Get-ChildItem -LiteralPath $Root -Recurse -Force -File | Sort-Object FullName | ForEach-Object {
        $Relative = Get-RelativePackagePath -Root $Root -Path $_.FullName
        $Map[$Relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return $Map
}

function Assert-NoReparsePoints {
    param([Parameter(Mandatory = $true)][string]$Root)
    Get-ChildItem -LiteralPath $Root -Force -Recurse | ForEach-Object {
        if (($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Tester package contains a reparse point: $($_.FullName)"
        }
    }
}

function Test-BytesForCredentialMaterial {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][bool]$Structured
    )
    $Ascii = [System.Text.Encoding]::ASCII.GetString($Bytes)
    $Utf16 = [System.Text.Encoding]::Unicode.GetString($Bytes)
    $HighConfidence = @(
        '(?is)-----BEGIN(?: [A-Z0-9]+)* PRIVATE KEY-----.{64,}-----END(?: [A-Z0-9]+)* PRIVATE KEY-----',
        '(?i)authorization\s*:\s*bearer\s+[A-Za-z0-9._~+/=-]{16,}',
        '(?i)(?:^|[^A-Za-z0-9])(ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|sk-[A-Za-z0-9_-]{20,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,})'
    )
    foreach ($Pattern in $HighConfidence) {
        if ($Ascii -match $Pattern -or $Utf16 -match $Pattern) { return $true }
    }
    if ($Structured) {
        $Assignment = '(?i)["'']?(?:api[_-]?key|password|session(?:[_-]?(?:id|token))?|access[_-]?token|refresh[_-]?token|csrf(?:[_-]?token)?|client[_-]?secret|private[_-]?key)["'']?\s*[:=]\s*["''][A-Za-z0-9+/=_.:-]{8,}["'']'
        if ($Ascii -match $Assignment -or $Utf16 -match $Assignment) { return $true }
    }
    return $false
}

function Test-FileForCredentialMaterial {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Structured
    )
    # Stream large debug executables and SQLite/attachment payloads instead of
    # materializing each file plus ASCII and UTF-16 copies at the same time.
    $ChunkSize = 4 * 1024 * 1024
    $OverlapSize = 128 * 1024
    $Buffer = New-Object byte[] $ChunkSize
    $Tail = New-Object byte[] 0
    $Stream = [System.IO.File]::OpenRead($Path)
    try {
        while (($Read = $Stream.Read($Buffer, 0, $Buffer.Length)) -gt 0) {
            $Window = New-Object byte[] ($Tail.Length + $Read)
            if ($Tail.Length -gt 0) {
                [System.Buffer]::BlockCopy($Tail, 0, $Window, 0, $Tail.Length)
            }
            [System.Buffer]::BlockCopy($Buffer, 0, $Window, $Tail.Length, $Read)
            if (Test-BytesForCredentialMaterial -Bytes $Window -Structured $Structured) {
                return $true
            }
            $TailLength = [System.Math]::Min($OverlapSize, $Window.Length)
            $Tail = New-Object byte[] $TailLength
            [System.Buffer]::BlockCopy($Window, $Window.Length - $TailLength, $Tail, 0, $TailLength)
        }
    }
    finally {
        $Stream.Dispose()
    }
    return $false
}

function Invoke-PackageSensitiveScan {
    param([Parameter(Mandatory = $true)][string]$PackageRoot)
    Assert-NoReparsePoints -Root $PackageRoot
    $ForbiddenExtensions = @('.key', '.pem', '.pfx', '.p12', '.ppk', '.jks', '.kdbx')
    $StructuredExtensions = @('.cfg', '.cmd', '.config', '.csv', '.html', '.ini', '.js', '.json', '.md', '.ps1', '.sha256', '.toml', '.ts', '.txt', '.xml', '.yaml', '.yml')
    $Scanned = 0
    $ScannedBytes = [int64]0
    Get-ChildItem -LiteralPath $PackageRoot -Recurse -Force -File | ForEach-Object {
        $LowerName = $_.Name.ToLowerInvariant()
        if ($LowerName -eq '.env' -or $LowerName.StartsWith('.env.') -or
            $ForbiddenExtensions -contains $_.Extension.ToLowerInvariant()) {
            throw "Forbidden secret-bearing file type in Tester package: $($_.Name)"
        }
        $Structured = $StructuredExtensions -contains $_.Extension.ToLowerInvariant()
        if (Test-FileForCredentialMaterial -Path $_.FullName -Structured $Structured) {
            throw "Credential-like material detected in Tester package file: $($_.Name)"
        }
        $Scanned += 1
        $ScannedBytes += $_.Length
    }
    return [pscustomobject]@{
        schemaVersion = 1
        status = 'PASS'
        scannedFiles = $Scanned
        scannedBytes = $ScannedBytes
        credentialMatches = 0
        reparsePoints = 0
        forbiddenExtensions = 0
        aiSecretInventoryVerifiedEmpty = $true
        inventoryVerifier = 'muriarc-standard-fixture verify'
        dataClassification = 'synthetic-standard-v1-only'
    }
}

if ([string]::IsNullOrWhiteSpace($BuildRoot)) {
    $BuildRoot = if (Test-Path -LiteralPath 'E:\Muriarc') {
        'E:\Muriarc\builds'
    } else {
        Join-Path $env:LOCALAPPDATA 'MuriArc\builds'
    }
}

$GitExe = Require-File (Join-Path $env:ProgramFiles 'Git\cmd\git.exe')
$RustBin = Join-Path $env:USERPROFILE '.cargo\bin'
$CargoExe = Require-File (Join-Path $RustBin 'cargo.exe')
$RustcExe = Require-File (Join-Path $RustBin 'rustc.exe')
$NodeDir = Join-Path $env:ProgramFiles 'nodejs'
$NodeExe = Require-File (Join-Path $NodeDir 'node.exe')
$WindowsPowerShellExe = Require-File (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe')
$CorepackJs = Require-File (Join-Path $NodeDir 'node_modules\corepack\dist\corepack.js')
$PnpmJs = Require-File (Join-Path $NodeDir 'node_modules\corepack\dist\pnpm.js')
$VsRoot = Require-Directory (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\2022\BuildTools')
$VcVars = Require-File (Join-Path $VsRoot 'VC\Auxiliary\Build\vcvars64.bat')

$VsStdout = Join-Path $env:TEMP ("muriarc-vsenv-" + [Guid]::NewGuid().ToString('N') + '.stdout')
$VsStderr = Join-Path $env:TEMP ("muriarc-vsenv-" + [Guid]::NewGuid().ToString('N') + '.stderr')
try {
    $VsArguments = "/d /s /c `"`"$VcVars`" && set`""
    $VsProcess = Start-Process -FilePath $env:ComSpec -ArgumentList $VsArguments `
        -NoNewWindow -Wait -PassThru -RedirectStandardOutput $VsStdout -RedirectStandardError $VsStderr
    if ($VsProcess.ExitCode -ne 0) {
        $Details = if (Test-Path -LiteralPath $VsStderr) { Get-Content -LiteralPath $VsStderr -Raw } else { '' }
        throw "vcvars64 initialization failed with exit code $($VsProcess.ExitCode): $Details"
    }
    foreach ($Line in Get-Content -LiteralPath $VsStdout) {
        if ($Line -match '^([^=]+)=(.*)$') {
            Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2]
        }
    }
}
finally {
    Remove-Item -LiteralPath $VsStdout, $VsStderr -Force -ErrorAction SilentlyContinue
}

# Some local BuildTools installations do not register their separately
# installed SDK with vswhere/vcvars. Add only the newest complete x64 SDK.
$SdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
$SdkVersion = Get-ChildItem -LiteralPath (Join-Path $SdkRoot 'Lib') -Directory |
    Where-Object {
        (Test-Path -LiteralPath (Join-Path $_.FullName 'um\x64\kernel32.lib') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $_.FullName 'ucrt\x64\ucrt.lib') -PathType Leaf)
    } | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
if (-not $SdkVersion) { throw 'A complete x64 Windows SDK is required.' }
$SdkInclude = Join-Path $SdkRoot "Include\$SdkVersion"
$SdkLib = Join-Path $SdkRoot "Lib\$SdkVersion"
$SdkBin = Join-Path $SdkRoot "bin\$SdkVersion\x64"
Require-File (Join-Path $SdkBin 'rc.exe') | Out-Null
$env:WindowsSdkDir = "$SdkRoot\"
$env:WindowsSDKVersion = "$SdkVersion\"
$env:UniversalCRTSdkDir = "$SdkRoot\"
$env:UCRTVersion = $SdkVersion
$env:INCLUDE = "$env:INCLUDE;$(Join-Path $SdkInclude 'ucrt');$(Join-Path $SdkInclude 'shared');$(Join-Path $SdkInclude 'um');$(Join-Path $SdkInclude 'winrt');$(Join-Path $SdkInclude 'cppwinrt')"
$env:LIB = "$env:LIB;$(Join-Path $SdkLib 'ucrt\x64');$(Join-Path $SdkLib 'um\x64')"
$env:LIBPATH = "$env:LIBPATH;$(Join-Path $SdkRoot "UnionMetadata\$SdkVersion");$(Join-Path $SdkRoot "References\$SdkVersion")"
$env:PATH = "$SdkBin;$RustBin;$NodeDir;$env:PATH"

$RepoRoot = Require-Directory $RepoRoot
$BuildRoot = if (Test-Path -LiteralPath $BuildRoot) {
    (Resolve-Path -LiteralPath $BuildRoot).Path
} else {
    New-Item -ItemType Directory -Path $BuildRoot -Force | Select-Object -ExpandProperty FullName
}
Set-Location -LiteralPath $RepoRoot
$env:GIT_CONFIG_COUNT = '1'
$env:GIT_CONFIG_KEY_0 = 'safe.directory'
$env:GIT_CONFIG_VALUE_0 = $RepoRoot

$OriginUrl = (Invoke-NativeCapture -FilePath $GitExe -Arguments @('remote', 'get-url', 'origin') -Step 'git origin URL').Stdout.TrimEnd('/')
if ($OriginUrl -notin @('https://github.com/jarxunlai/MuriArc.git', 'https://github.com/jarxunlai/MuriArc')) {
    throw "Tester source origin is not the canonical GitHub repository: $OriginUrl"
}
Invoke-Native -FilePath $GitExe -Arguments @(
    'fetch', '--no-tags', '--prune', 'origin',
    '+refs/heads/main:refs/remotes/origin/main'
) -Step 'fetch canonical origin/main'
$ActualCommit = (Invoke-NativeCapture -FilePath $GitExe -Arguments @('rev-parse', 'HEAD') -Step 'git rev-parse HEAD').Stdout
$OriginMain = (Invoke-NativeCapture -FilePath $GitExe -Arguments @('rev-parse', 'refs/remotes/origin/main') -Step 'git origin/main identity').Stdout
$Dirty = (Invoke-NativeCapture -FilePath $GitExe -Arguments @('status', '--porcelain=v1', '--untracked-files=all') -Step 'git clean-tree check').Stdout
if ($ActualCommit -ne $ExpectedCommit) {
    throw "Commit mismatch: expected $ExpectedCommit, got $ActualCommit"
}
if ($OriginMain -ne $ExpectedCommit) {
    throw "Tester source must equal the freshly fetched origin/main tip: expected $ExpectedCommit, got $OriginMain"
}
if (-not [string]::IsNullOrWhiteSpace($Dirty)) {
    throw "Tester source tree is not clean:`n$Dirty"
}

$ShortCommit = $ExpectedCommit.Substring(0, 12)
$Identifier = "org.muriarc.desktop.tester.c$ShortCommit"
$RunId = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $ShortCommit
$RunRoot = Join-Path $BuildRoot "windows-tester\$RunId"
if (Test-Path -LiteralPath $RunRoot) { throw "Tester run root already exists: $RunRoot" }
$PackageRoot = Join-Path $RunRoot "MuriArc-1.0.0-standard-v1-tester-$ShortCommit"
$EvidenceRoot = Join-Path $RunRoot 'evidence'
$AppRoot = Join-Path $PackageRoot 'app'
$DataRoot = Join-Path $PackageRoot 'data-root'
$FixturePackageRoot = Join-Path $PackageRoot 'fixture-definition'
$env:CARGO_TARGET_DIR = Join-Path $BuildRoot 'cargo-target\windows-tester-shared'
New-Item -ItemType Directory -Path $AppRoot, $EvidenceRoot, $env:CARGO_TARGET_DIR -Force | Out-Null

# Do not let inherited frontend variables or credentials become compiler input
# or accidentally appear in logs and generated assets. Public dependencies are
# used, so the Tester build does not need registry credentials.
$SensitiveEnvironment = @(Get-ChildItem Env: | Where-Object {
    $_.Name -like 'VITE_*' -or
    $_.Name -match '(?i)(?:^|_)(?:api_?key|credential|csrf|master_?key|password|passwd|private_?key|secret|session|token)(?:$|_)'
})
$SanitizedEnvironmentVariableCount = $SensitiveEnvironment.Count
$SensitiveEnvironment | ForEach-Object {
    Remove-Item -LiteralPath "Env:$($_.Name)" -ErrorAction SilentlyContinue
}
$env:CI = 'true'
$env:VITE_MURIARC_GATEWAY = 'local'
Remove-Item Env:MURIARC_DESKTOP_UPDATER_PUBLIC_KEY -ErrorAction SilentlyContinue
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue

$Transcript = Join-Path $EvidenceRoot 'build.log'
Start-Transcript -Path $Transcript | Out-Null
try {
    Write-Output "expected_commit=$ExpectedCommit"
    Write-Output "actual_commit=$ActualCommit"
    Write-Output "origin_main=$OriginMain"
    Write-Output "origin_url=$OriginUrl"
    Write-Output "identifier=$Identifier"
    Write-Output "run_root=$RunRoot"
    Write-Output 'classification=unsigned synthetic-data not-for-production'

    $RustcVersion = (Invoke-NativeCapture -FilePath $RustcExe -Arguments @('--version', '--verbose') -Step 'rustc version').Stdout
    $CargoVersion = (Invoke-NativeCapture -FilePath $CargoExe -Arguments @('--version', '--verbose') -Step 'cargo version').Stdout
    $NodeVersion = (Invoke-NativeCapture -FilePath $NodeExe -Arguments @('--version') -Step 'node version').Stdout
    $WindowsPowerShellVersion = (Invoke-NativeCapture -FilePath $WindowsPowerShellExe -Arguments @('-NoProfile', '-Command', '$PSVersionTable.PSVersion.ToString()') -Step 'Windows PowerShell version').Stdout
    if ($WindowsPowerShellVersion -notlike '5.1.*') {
        throw "Windows PowerShell 5.1 is required to validate the friend launcher, got $WindowsPowerShellVersion"
    }
    $RustcHost = (($RustcVersion -split "`r?`n") | Where-Object { $_ -like 'host: *' } | Select-Object -First 1) -replace '^host:\s*', ''
    if ($RustcHost -ne 'x86_64-pc-windows-msvc') {
        throw "The Tester requires the x86_64-pc-windows-msvc Rust host, got $RustcHost"
    }
    $NodeArchitecture = (Invoke-NativeCapture -FilePath $NodeExe -Arguments @('-p', 'process.arch') -Step 'node architecture').Stdout
    if ($NodeArchitecture -ne 'x64') {
        throw "The Tester requires x64 Node.js, got $NodeArchitecture"
    }
    Invoke-Native -FilePath $NodeExe -Arguments @($CorepackJs, 'prepare', 'pnpm@11.5.0', '--activate') -Step 'activate pnpm 11.5.0'
    $PnpmVersion = (Invoke-NativeCapture -FilePath $NodeExe -Arguments @($PnpmJs, '--version') -Step 'pnpm version').Stdout
    if ($PnpmVersion -ne '11.5.0') { throw "pnpm 11.5.0 is required, got $PnpmVersion" }

    Invoke-Native -FilePath $NodeExe -Arguments @($PnpmJs, '--dir', 'ui', 'install', '--frozen-lockfile') -Step 'pnpm install'
    Invoke-Native -FilePath $CargoExe -Arguments @('fmt', '--all', '--', '--check') -Step 'cargo fmt'
    Invoke-Native -FilePath $CargoExe -Arguments @('test', '--locked', '-p', 'muriarc-standard-fixture', '--all-targets') -Step 'standard fixture tests'
    Invoke-Native -FilePath $CargoExe -Arguments @('clippy', '--locked', '-p', 'muriarc-standard-fixture', '--all-targets', '--', '-D', 'warnings') -Step 'standard fixture clippy'
    Invoke-Native -FilePath $CargoExe -Arguments @('clippy', '--locked', '-p', 'muriarc-desktop', '--all-targets', '--all-features', '--', '-D', 'warnings') -Step 'Desktop strict clippy'
    Invoke-Native -FilePath $CargoExe -Arguments @('test', '--locked', '-p', 'muriarc-desktop', '--all-features') -Step 'Desktop tests'
    Invoke-Native -FilePath $NodeExe -Arguments @($PnpmJs, '--dir', 'ui', 'run', 'test') -Step 'UI tests'
    Invoke-Native -FilePath $NodeExe -Arguments @($PnpmJs, '--dir', 'ui', 'run', 'typecheck') -Step 'UI typecheck'
    Invoke-Native -FilePath $NodeExe -Arguments @($PnpmJs, '--dir', 'ui', 'run', 'build') -Step 'local UI production build'

    $TauriJs = Require-File (Join-Path $RepoRoot 'ui\node_modules\@tauri-apps\cli\tauri.js')
    $TauriOverride = Join-Path $EvidenceRoot 'tauri-tester.json'
    Write-Json -Path $TauriOverride -Value ([pscustomobject]@{
        productName = 'MuriArc 1.0 Standard-v1 Tester'
        identifier = $Identifier
        build = @{ beforeBuildCommand = '' }
        bundle = @{ active = $false; createUpdaterArtifacts = $false }
    })
    Invoke-Native -FilePath $NodeExe -Arguments @($TauriJs, 'build', '--debug', '--no-bundle', '--config', $TauriOverride) -Step 'Tauri isolated Tester build'

    $BuiltExecutable = Require-File (Join-Path $env:CARGO_TARGET_DIR 'debug\MuriArc.exe')
    $ExecutableName = "MuriArc-1.0.0-standard-v1-tester-$ShortCommit-debug.exe"
    $PublishedExecutable = Join-Path $AppRoot $ExecutableName
    Copy-Item -LiteralPath $BuiltExecutable -Destination $PublishedExecutable
    $ExecutableHash = (Get-FileHash -LiteralPath $PublishedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()

    $FixtureRoot = Require-Directory (Join-Path $RepoRoot 'fixtures\standard-v1')
    $Seed = Invoke-NativeCapture -FilePath $PublishedExecutable -Arguments @(
        '--muriarc-standard-fixture', 'seed', '--fixture', $FixtureRoot,
        '--output', $DataRoot, '--source-commit', $ExpectedCommit
    ) -Step 'Desktop standard-v1 seed'
    $SeedReceipt = $Seed.Stdout | ConvertFrom-Json
    if ($SeedReceipt.status -ne 'PASS' -or
        $SeedReceipt.sourceCommit -ne $ExpectedCommit -or
        $SeedReceipt.applicationVersion -ne '1.0.0' -or
        $SeedReceipt.dataEpoch -ne 'E0001' -or
        $SeedReceipt.backend -ne 'sqlite') {
        throw 'Desktop standard-v1 seed receipt identity is invalid.'
    }
    $Verify = Invoke-NativeCapture -FilePath $PublishedExecutable -Arguments @(
        '--muriarc-standard-fixture', 'verify', '--fixture', $FixtureRoot,
        '--output', $DataRoot, '--source-commit', $ExpectedCommit
    ) -Step 'Desktop standard-v1 verify'
    $VerifyReceipt = $Verify.Stdout | ConvertFrom-Json
    if (($VerifyReceipt | ConvertTo-Json -Depth 12 -Compress) -ne ($SeedReceipt | ConvertTo-Json -Depth 12 -Compress)) {
        throw 'Desktop seed and verify receipts differ.'
    }
    Write-Json -Path (Join-Path $EvidenceRoot 'desktop-standard-v1-seed-receipt.json') -Value $SeedReceipt
    Write-Json -Path (Join-Path $EvidenceRoot 'desktop-standard-v1-verify-receipt.json') -Value $VerifyReceipt
    New-Item -ItemType Directory -Path $FixturePackageRoot | Out-Null
    Get-ChildItem -LiteralPath $FixtureRoot -Force | Copy-Item -Destination $FixturePackageRoot -Recurse -Force

    # Prove the packaged data can start the exact Desktop executable without
    # mutating the packaged baseline.
    $PackagedDataBeforeSmoke = Get-FileDigestMap -Root $DataRoot
    $SmokeRoot = Join-Path $env:TEMP ("muriarc-tester-smoke-" + [Guid]::NewGuid().ToString('N'))
    $SmokeData = Join-Path $SmokeRoot 'data-root'
    $SmokeApp = Join-Path $SmokeRoot 'app'
    $ConfigRoot = Join-Path ([Environment]::GetFolderPath('ApplicationData')) $Identifier
    if (Test-Path -LiteralPath $ConfigRoot) {
        throw "Tester smoke config already exists and will not be overwritten: $ConfigRoot"
    }
    $SmokeProcess = $null
    try {
        New-Item -ItemType Directory -Path $SmokeData, $SmokeApp, $ConfigRoot -Force | Out-Null
        Get-ChildItem -LiteralPath $DataRoot -Force | Copy-Item -Destination $SmokeData -Recurse -Force
        $SmokeExecutable = Join-Path $SmokeApp $ExecutableName
        Copy-Item -LiteralPath $PublishedExecutable -Destination $SmokeExecutable
        Write-Utf8NoBom -Path (Join-Path $ConfigRoot 'storage-location.json') -Value (([pscustomobject]@{
            version = 1
            activeDataRoot = $SmokeData
        } | ConvertTo-Json) + "`n")
        $SmokeStdout = Join-Path $EvidenceRoot 'smoke.stdout.log'
        $SmokeStderr = Join-Path $EvidenceRoot 'smoke.stderr.log'
        $SmokeProcess = Start-Process -FilePath $SmokeExecutable -WorkingDirectory $SmokeApp -PassThru `
            -RedirectStandardOutput $SmokeStdout -RedirectStandardError $SmokeStderr
        Start-Sleep -Seconds 15
        if ($SmokeProcess.HasExited) {
            throw "Desktop Tester smoke exited early with code $($SmokeProcess.ExitCode)"
        }
        Stop-Process -Id $SmokeProcess.Id -Force
        $SmokeProcess.WaitForExit()
        Write-Json -Path (Join-Path $EvidenceRoot 'smoke-summary.json') -Value ([pscustomobject]@{
            schemaVersion = 1
            status = 'PASS'
            durationSeconds = 15
            remainedRunningUntilTestStop = $true
            usedTemporaryDataCopy = $true
            packagedDataWasNotMutated = $true
        })
    }
    finally {
        if ($SmokeProcess -and -not $SmokeProcess.HasExited) {
            Stop-Process -Id $SmokeProcess.Id -Force -ErrorAction SilentlyContinue
        }
        Remove-Item -LiteralPath $ConfigRoot -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $SmokeRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    $PackagedDataAfterSmoke = Get-FileDigestMap -Root $DataRoot
    if (($PackagedDataBeforeSmoke | ConvertTo-Json -Compress) -ne
        ($PackagedDataAfterSmoke | ConvertTo-Json -Compress)) {
        throw 'Desktop startup smoke mutated the packaged synthetic baseline.'
    }

    $LauncherTemplate = @'
[CmdletBinding()]
param([switch]$VerifyOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PackageRoot = $PSScriptRoot
$Identifier = '__IDENTIFIER__'
$Commit = '__COMMIT__'
$ExecutableName = '__EXECUTABLE__'
$SourceData = Join-Path $PackageRoot 'data-root'
$FixtureDefinition = Join-Path $PackageRoot 'fixture-definition'
$Executable = Join-Path $PackageRoot "app\$ExecutableName"
$ChecksumFile = Join-Path $PackageRoot 'CHECKSUMS.sha256'
$ManifestFile = Join-Path $PackageRoot 'TESTER-MANIFEST.json'
$PackageRootItem = Get-Item -LiteralPath $PackageRoot -Force
if (($PackageRootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'Tester package root must not be a reparse point.'
}
$CanonicalPackageRoot = $PackageRootItem.FullName.TrimEnd('\', '/')
$CanonicalPackagePrefix = $CanonicalPackageRoot + [System.IO.Path]::DirectorySeparatorChar
$PackageEntries = @(Get-ChildItem -LiteralPath $PackageRoot -Recurse -Force)
foreach ($Entry in $PackageEntries) {
    if (($Entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Tester package contains a reparse point: $($Entry.FullName)"
    }
}

function Resolve-CheckedPackageFile {
    param([Parameter(Mandatory = $true)][string]$RelativePath)
    if ([System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "Checksum contains a rooted path: $RelativePath"
    }
    $Segments = $RelativePath -split '[\\/]'
    if ($Segments.Count -eq 0 -or $Segments -contains '' -or
        $Segments -contains '.' -or $Segments -contains '..') {
        throw "Checksum contains an unsafe relative path: $RelativePath"
    }
    $CandidatePath = [System.IO.Path]::GetFullPath((Join-Path $PackageRoot ($RelativePath.Replace('/', '\'))))
    if (-not (Test-Path -LiteralPath $CandidatePath -PathType Leaf)) {
        throw "Tester package file is missing: $RelativePath"
    }
    $FullPath = (Get-Item -LiteralPath $CandidatePath -Force).FullName
    if (-not $FullPath.StartsWith($CanonicalPackagePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Checksum path escapes the Tester package: $RelativePath"
    }
    return $FullPath
}

function ConvertTo-LauncherNativeArgument {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)
    if ($Value.Length -gt 0 -and $Value -notmatch '[\s&|<>^"]') {
        return $Value
    }
    $Builder = New-Object System.Text.StringBuilder
    $Backslash = [char]92
    [void]$Builder.Append('"')
    $PendingBackslashes = 0
    foreach ($Character in $Value.ToCharArray()) {
        if ($Character -eq $Backslash) {
            $PendingBackslashes += 1
            continue
        }
        if ($Character -eq '"') {
            [void]$Builder.Append($Backslash, ($PendingBackslashes * 2) + 1)
            [void]$Builder.Append('"')
        } else {
            [void]$Builder.Append($Backslash, $PendingBackslashes)
            [void]$Builder.Append($Character)
        }
        $PendingBackslashes = 0
    }
    [void]$Builder.Append($Backslash, $PendingBackslashes * 2)
    [void]$Builder.Append('"')
    return $Builder.ToString()
}

if (-not (Test-Path -LiteralPath $ChecksumFile -PathType Leaf)) {
    throw 'CHECKSUMS.sha256 is missing.'
}
$CanonicalChecksumFile = (Get-Item -LiteralPath $ChecksumFile -Force).FullName
$ExpectedFiles = @{}
foreach ($Line in Get-Content -LiteralPath $ChecksumFile -Encoding UTF8) {
    if ([string]::IsNullOrWhiteSpace($Line)) { continue }
    if ($Line -notmatch '^([0-9a-fA-F]{64}) \*(.+)$') {
        throw "Invalid CHECKSUMS.sha256 line: $Line"
    }
    $ExpectedHash = $Matches[1].ToLowerInvariant()
    $RelativePath = $Matches[2]
    $CheckedPath = Resolve-CheckedPackageFile -RelativePath $RelativePath
    $Key = $CheckedPath.ToLowerInvariant()
    if ($ExpectedFiles.ContainsKey($Key)) {
        throw "Duplicate CHECKSUMS.sha256 path: $RelativePath"
    }
    $ActualHash = (Get-FileHash -LiteralPath $CheckedPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($ActualHash -ne $ExpectedHash) {
        throw "Tester package checksum mismatch: $RelativePath"
    }
    $ExpectedFiles[$Key] = $true
}
$ActualFiles = @($PackageEntries | Where-Object {
    -not $_.PSIsContainer -and
    -not [string]::Equals($_.FullName, $CanonicalChecksumFile, [System.StringComparison]::OrdinalIgnoreCase)
})
if ($ActualFiles.Count -ne $ExpectedFiles.Count) {
    throw "Tester package file inventory differs from CHECKSUMS.sha256."
}
foreach ($File in $ActualFiles) {
    if (-not $ExpectedFiles.ContainsKey($File.FullName.ToLowerInvariant())) {
        throw "Unexpected file in Tester package: $($File.FullName)"
    }
}

$Manifest = Get-Content -LiteralPath $ManifestFile -Raw -Encoding UTF8 | ConvertFrom-Json
if ($Manifest.status -ne 'PASS' -or
    $Manifest.version -ne '1.0.0' -or
    $Manifest.dataEpoch -ne 'E0001' -or
    $Manifest.sourceCommit -ne $Commit -or
    $Manifest.sourceBranch -ne 'main' -or
    $Manifest.identifier -ne $Identifier -or
    $Manifest.platform -ne 'windows-x86_64' -or
    $Manifest.executable -ne "app/$ExecutableName" -or
    $Manifest.unsigned -ne $true -or
    $Manifest.syntheticData -ne $true -or
    $Manifest.notForProduction -ne $true -or
    $Manifest.formalRelease -ne $false -or
    $Manifest.formalRcEvidence -ne $false -or
    $Manifest.aiSecretInventoryVerifiedEmpty -ne $true) {
    throw 'TESTER-MANIFEST.json identity or safety classification is invalid.'
}
$ExecutableHash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ExecutableHash -ne $Manifest.executableSha256) {
    throw 'Tester executable does not match TESTER-MANIFEST.json.'
}

$FixtureVerifyArguments = @(
    '--muriarc-standard-fixture', 'verify', '--fixture', $FixtureDefinition,
    '--output', $SourceData, '--source-commit', $Commit
)
$FixtureVerifyArgumentLine = ($FixtureVerifyArguments | ForEach-Object {
    ConvertTo-LauncherNativeArgument $_
}) -join ' '
$FixtureVerifyProcess = Start-Process -FilePath $Executable -ArgumentList $FixtureVerifyArgumentLine `
    -WorkingDirectory (Join-Path $PackageRoot 'app') -NoNewWindow -Wait -PassThru
if ($FixtureVerifyProcess.ExitCode -ne 0) {
    throw "Packaged synthetic baseline verification failed with exit code $($FixtureVerifyProcess.ExitCode)."
}
if ($VerifyOnly) {
    Write-Output 'MURIARC_TESTER_LAUNCHER_VERIFY=PASS'
    return
}
$RuntimeParent = Join-Path $env:LOCALAPPDATA "MuriArc\Tester\$Commit"
$RuntimeData = Join-Path $RuntimeParent 'data-root'
if (-not (Test-Path -LiteralPath $RuntimeData -PathType Container)) {
    New-Item -ItemType Directory -Path $RuntimeParent -Force | Out-Null
    $Staging = Join-Path $RuntimeParent ('.data-root.staging-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $Staging | Out-Null
        Get-ChildItem -LiteralPath $SourceData -Force | Copy-Item -Destination $Staging -Recurse -Force
        Move-Item -LiteralPath $Staging -Destination $RuntimeData
    }
    finally {
        Remove-Item -LiteralPath $Staging -Recurse -Force -ErrorAction SilentlyContinue
    }
}
$ConfigRoot = Join-Path ([Environment]::GetFolderPath('ApplicationData')) $Identifier
New-Item -ItemType Directory -Path $ConfigRoot -Force | Out-Null
$Locator = [pscustomobject]@{ version = 1; activeDataRoot = $RuntimeData }
$Encoding = New-Object System.Text.UTF8Encoding($false)
$LocatorPath = Join-Path $ConfigRoot 'storage-location.json'
$LocatorTemporary = Join-Path $ConfigRoot ('.storage-location-' + [Guid]::NewGuid().ToString('N') + '.tmp')
[System.IO.File]::WriteAllText($LocatorTemporary, ($Locator | ConvertTo-Json), $Encoding)
Move-Item -LiteralPath $LocatorTemporary -Destination $LocatorPath -Force
Start-Process -FilePath $Executable -WorkingDirectory (Join-Path $PackageRoot 'app')
'@
    $Launcher = $LauncherTemplate.Replace('__IDENTIFIER__', $Identifier).Replace('__COMMIT__', $ExpectedCommit).Replace('__EXECUTABLE__', $ExecutableName)
    Write-Utf8NoBom -Path (Join-Path $PackageRoot 'Start-MuriArc-Tester.ps1') -Value ($Launcher + "`n")
    $CmdLauncher = @'
@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-MuriArc-Tester.ps1"
if errorlevel 1 (
  echo MuriArc Tester failed. Review the error above.
  pause
  exit /b 1
)
'@
    Write-Utf8NoBom -Path (Join-Path $PackageRoot 'Start-MuriArc-Tester.cmd') -Value ($CmdLauncher -replace "`r?`n", "`r`n")

    $ReadmeSource = Require-File (Join-Path $RepoRoot 'scripts\windows-tester\README-TESTER-zh-CN.txt')
    Copy-Item -LiteralPath $ReadmeSource -Destination (Join-Path $PackageRoot 'README-TESTER-zh-CN.txt')

    Write-Json -Path (Join-Path $PackageRoot 'TESTER-MANIFEST.json') -Value ([pscustomobject]@{
        schemaVersion = 1
        status = 'PASS'
        product = 'MuriArc'
        version = '1.0.0'
        dataEpoch = 'E0001'
        sourceCommit = $ExpectedCommit
        sourceBranch = 'main'
        sourceOrigin = 'https://github.com/jarxunlai/MuriArc'
        identifier = $Identifier
        platform = 'windows-x86_64'
        executable = "app/$ExecutableName"
        executableSha256 = $ExecutableHash
        fixtureDefinition = 'fixture-definition'
        datasetId = $SeedReceipt.datasetId
        datasetVersion = $SeedReceipt.datasetVersion
        datasetSha256 = $SeedReceipt.datasetSha256
        generationId = $SeedReceipt.generationId
        unsigned = $true
        syntheticData = $true
        notForProduction = $true
        formalRelease = $false
        formalRcEvidence = $false
        aiSecretInventoryVerifiedEmpty = $true
    })

    $Checksums = Get-FileDigestMap -Root $PackageRoot
    $ChecksumLines = $Checksums.GetEnumerator() | ForEach-Object { "$($_.Value) *$($_.Key)" }
    Write-Utf8NoBom -Path (Join-Path $PackageRoot 'CHECKSUMS.sha256') -Value (($ChecksumLines -join "`n") + "`n")

    $SensitiveScan = Invoke-PackageSensitiveScan -PackageRoot $PackageRoot
    Write-Json -Path (Join-Path $EvidenceRoot 'sensitive-scan.json') -Value $SensitiveScan

    $PackageBeforeLauncherVerification = Get-FileDigestMap -Root $PackageRoot
    $LauncherVerification = Invoke-NativeCapture -FilePath $WindowsPowerShellExe -Arguments @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        (Join-Path $PackageRoot 'Start-MuriArc-Tester.ps1'), '-VerifyOnly'
    ) -Step 'packaged Tester launcher verification'
    if ($LauncherVerification.Stdout -notmatch '(?m)^MURIARC_TESTER_LAUNCHER_VERIFY=PASS$') {
        throw 'Packaged Tester launcher did not emit its PASS receipt.'
    }
    Write-Utf8NoBom -Path (Join-Path $EvidenceRoot 'packaged-launcher-verify.log') -Value ($LauncherVerification.Stdout + "`n")
    $PackageAfterLauncherVerification = Get-FileDigestMap -Root $PackageRoot
    if (($PackageBeforeLauncherVerification | ConvertTo-Json -Compress) -ne
        ($PackageAfterLauncherVerification | ConvertTo-Json -Compress)) {
        throw 'Packaged Tester launcher verification mutated the package.'
    }

    $ArchiveName = "MuriArc-1.0.0-standard-v1-tester-$ShortCommit-windows-x64.zip"
    $ArchivePath = Join-Path $RunRoot $ArchiveName
    Compress-Archive -Path (Join-Path $PackageRoot '*') -DestinationPath $ArchivePath -CompressionLevel Optimal
    $ArchiveHash = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom -Path "$ArchivePath.sha256" -Value "$ArchiveHash *$ArchiveName`n"

    $Extracted = Join-Path $env:TEMP ("muriarc-tester-verify-" + [Guid]::NewGuid().ToString('N'))
    try {
        Expand-Archive -LiteralPath $ArchivePath -DestinationPath $Extracted
        Assert-NoReparsePoints -Root $Extracted
        $ExpectedFiles = Get-FileDigestMap -Root $PackageRoot
        $ActualFiles = Get-FileDigestMap -Root $Extracted
        if (($ExpectedFiles | ConvertTo-Json -Compress) -ne ($ActualFiles | ConvertTo-Json -Compress)) {
            throw 'Expanded Tester ZIP differs from the staged package.'
        }
    }
    finally {
        Remove-Item -LiteralPath $Extracted -Recurse -Force -ErrorAction SilentlyContinue
    }

    $PostBuildDirty = (Invoke-NativeCapture -FilePath $GitExe -Arguments @('status', '--porcelain=v1', '--untracked-files=all') -Step 'post-build clean-tree check').Stdout
    if (-not [string]::IsNullOrWhiteSpace($PostBuildDirty)) {
        throw "Tester build dirtied the source tree:`n$PostBuildDirty"
    }

    $Summary = [pscustomobject]@{
        schemaVersion = 1
        status = 'PASS'
        sourceCommit = $ExpectedCommit
        originMain = $OriginMain
        cleanTreeBeforeAndAfter = $true
        identifier = $Identifier
        archive = $ArchiveName
        archiveBytes = (Get-Item -LiteralPath $ArchivePath).Length
        archiveSha256 = $ArchiveHash
        packageFileCount = (Get-ChildItem -LiteralPath $PackageRoot -Recurse -Force -File).Count
        datasetSha256 = $SeedReceipt.datasetSha256
        generationId = $SeedReceipt.generationId
        aiSecretInventoryVerifiedEmpty = $true
        checks = @(
            'cargo-fmt', 'standard-fixture-tests', 'standard-fixture-clippy',
            'desktop-strict-clippy', 'desktop-tests', 'ui-tests', 'ui-typecheck',
            'ui-local-build', 'tauri-debug-no-bundle', 'desktop-seed',
            'desktop-verify', 'desktop-startup-smoke', 'sensitive-scan',
            'packaged-launcher-verify', 'archive-round-trip'
        )
        classification = @('unsigned', 'synthetic-data', 'not-for-production')
        formalRelease = $false
        formalRcEvidence = $false
        testerTagSuggestion = "tester-v1.0.0-standard-v1-$ShortCommit"
        toolchain = @{
            rustc = ($RustcVersion -split "`r?`n")[0]
            cargo = ($CargoVersion -split "`r?`n")[0]
            node = $NodeVersion
            nodeArchitecture = $NodeArchitecture
            pnpm = $PnpmVersion
            rustcHost = $RustcHost
            windowsPowerShell = $WindowsPowerShellVersion
            windowsSdk = $SdkVersion
        }
        sanitizedEnvironmentVariableCount = $SanitizedEnvironmentVariableCount
    }
    Write-Json -Path (Join-Path $EvidenceRoot 'tester-package-summary.json') -Value $Summary
    Write-Json -Path (Join-Path $RunRoot "$ArchiveName.manifest.json") -Value $Summary

    Write-Output 'MURIARC_WINDOWS_TESTER_BUILD=PASS'
    Write-Output "archive=$ArchivePath"
    Write-Output "sha256=$ArchiveHash"
    Write-Output "manifest=$(Join-Path $RunRoot "$ArchiveName.manifest.json")"
}
finally {
    Stop-Transcript | Out-Null
}
