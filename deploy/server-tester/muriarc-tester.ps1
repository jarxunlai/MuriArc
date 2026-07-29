[CmdletBinding()]
param(
    [Parameter(Position = 0, Mandatory = $true)]
    [ValidateSet('verify', 'init-empty', 'init-demo', 'up', 'status', 'logs', 'down')]
    [string]$Command,
    [string]$EnvFile = (Join-Path $PSScriptRoot '.env')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$ComposeFile = Join-Path $PSScriptRoot 'compose.yaml'
$BootstrapFile = Join-Path $PSScriptRoot 'compose.bootstrap.yaml'

function Fail([string]$Message) { throw $Message }

function Invoke-NativeChecked {
    param([string]$FilePath, [string[]]$Arguments)
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) { Fail "$FilePath failed with exit code $LASTEXITCODE" }
}

function Get-DotEnvValue([string]$Key) {
    if (-not (Test-Path -LiteralPath $EnvFile -PathType Leaf)) { Fail "Missing environment file: $EnvFile" }
    $item = Get-Item -LiteralPath $EnvFile -Force
    if ($item.LinkType) { Fail "Environment file must not be a link: $EnvFile" }
    $matches = @(Get-Content -LiteralPath $EnvFile | Where-Object { $_ -match ('^{0}=' -f [regex]::Escape($Key)) })
    if ($matches.Count -ne 1) { Fail "$Key must appear exactly once in $EnvFile" }
    $value = ($matches[0] -split '=', 2)[1]
    if ([string]::IsNullOrWhiteSpace($value)) { Fail "$Key must not be empty" }
    return $value.TrimEnd("`r")
}

function Test-Environment {
    $required = @(
        'MURIARC_TESTER_DATASET_MODE', 'MURIARC_COMPOSE_PROJECT_NAME',
        'MURIARC_TESTER_SOURCE_COMMIT', 'MURIARC_TESTER_SERVER_PORT',
        'MURIARC_POSTGRES_DB', 'MURIARC_POSTGRES_USER', 'MURIARC_POSTGRES_PASSWORD',
        'MURIARC_DATA_ROOT', 'MURIARC_ATTACHMENT_ROOT', 'MURIARC_AI_MASTER_KEY_FILE',
        'MURIARC_LAB_ID', 'MURIARC_LAB_NAME', 'MURIARC_ROOT_USER_ID',
        'MURIARC_ROOT_USER_EMAIL', 'MURIARC_ROOT_USER_NAME', 'MURIARC_ROOT_PASSWORD',
        'MURIARC_SESSION_COOKIE_SECURE', 'MURIARC_SESSION_TTL_HOURS'
    )
    foreach ($key in $required) { [void](Get-DotEnvValue $key) }
    $text = Get-Content -LiteralPath $EnvFile -Raw
    if ($text -match '(?m)(^|=)(REPLACE_|@@|<[^>]+>)') { Fail 'Replace every placeholder before continuing.' }
    if ($text -match '(?m)^MURIARC_(AI_MASTER_KEY|BOOTSTRAP_TOKEN|BOOTSTRAP_MCP_TOKEN)=') {
        Fail 'Shared AI/bootstrap secrets are forbidden in the Tester environment file.'
    }

    $script:Project = Get-DotEnvValue 'MURIARC_COMPOSE_PROJECT_NAME'
    $script:Mode = Get-DotEnvValue 'MURIARC_TESTER_DATASET_MODE'
    $script:Port = [int](Get-DotEnvValue 'MURIARC_TESTER_SERVER_PORT')
    $script:SourceCommit = Get-DotEnvValue 'MURIARC_TESTER_SOURCE_COMMIT'
    if ($Project -notmatch '^[a-z0-9][a-z0-9_-]{2,62}$') { Fail 'Invalid Compose project name.' }
    if ($Mode -notin @('empty', 'demo')) { Fail 'Dataset mode must be empty or demo.' }
    if ($Port -lt 1024 -or $Port -gt 65535) { Fail 'Tester port must be 1024..65535.' }
    if ($SourceCommit -notmatch '^[0-9a-f]{40}$') { Fail 'Source commit must be 40 lowercase hex characters.' }
    if ((Get-DotEnvValue 'MURIARC_LAB_ID') -eq (Get-DotEnvValue 'MURIARC_ROOT_USER_ID')) {
        Fail 'Lab ID and Root user ID must differ.'
    }
    if ((Get-DotEnvValue 'MURIARC_POSTGRES_PASSWORD') -notmatch '^[A-Za-z0-9_-]{32,}$') {
        Fail 'PostgreSQL password must be at least 32 URL-safe characters.'
    }
    if ((Get-DotEnvValue 'MURIARC_ROOT_PASSWORD') -notmatch '^[A-Za-z0-9_-]{32,}$') {
        Fail 'Root password must be at least 32 URL-safe characters.'
    }
    $ttl = [int](Get-DotEnvValue 'MURIARC_SESSION_TTL_HOURS')
    if ($ttl -lt 1 -or $ttl -gt 720) { Fail 'Session TTL must be 1..720 hours.' }
    if ((Get-DotEnvValue 'MURIARC_SESSION_COOKIE_SECURE') -notin @('true', 'false')) {
        Fail 'Cookie secure must be true or false.'
    }
}

function Invoke-Compose([string[]]$Arguments) {
    Invoke-NativeChecked 'docker' (@('compose', '--env-file', $EnvFile, '--project-name', $Project, '--file', $ComposeFile) + $Arguments)
}

function Invoke-ComposeBootstrap([string[]]$Arguments) {
    Invoke-NativeChecked 'docker' (@('compose', '--env-file', $EnvFile, '--project-name', $Project,
        '--file', $ComposeFile, '--file', $BootstrapFile) + $Arguments)
}

function Test-VolumeExists([string]$Name) {
    & docker volume inspect $Name *> $null
    return $LASTEXITCODE -eq 0
}

function Assert-Fresh {
    foreach ($volume in @("${Project}_postgres_data", "${Project}_server_data")) {
        if (Test-VolumeExists $volume) { Fail "Refusing initialization: volume already exists: $volume" }
    }
    $ids = & docker compose --env-file $EnvFile --project-name $Project --file $ComposeFile ps --all --quiet
    if ($LASTEXITCODE -ne 0) { Fail 'Could not inspect Compose resources.' }
    if (-not [string]::IsNullOrWhiteSpace(($ids -join ''))) { Fail 'Compose resources already exist.' }
}

function Assert-Initialized {
    foreach ($volume in @("${Project}_postgres_data", "${Project}_server_data")) {
        if (-not (Test-VolumeExists $volume)) { Fail 'Deployment is not initialized; use init-empty or init-demo.' }
    }
}

function Wait-Ready {
    $url = "http://127.0.0.1:$Port/readyz"
    for ($i = 0; $i -lt 60; $i++) {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $url -TimeoutSec 5
            if ($response.StatusCode -eq 200) { Write-Host "Ready: $url"; return }
        } catch { Start-Sleep -Seconds 2 }
    }
    Invoke-Compose @('ps')
    Fail "Server did not become ready at $url"
}

function Test-Checksums {
    $path = Join-Path $PSScriptRoot 'CHECKSUMS.sha256'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Fail 'CHECKSUMS.sha256 is missing.' }
    foreach ($line in Get-Content -LiteralPath $path) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^([0-9a-f]{64})  ([^/\\].*)$') { Fail 'Invalid checksum entry.' }
        $expected = $Matches[1]; $relative = $Matches[2]
        if ($relative.Contains('..') -or $relative.Contains('\')) { Fail "Unsafe checksum path: $relative" }
        $target = Join-Path $PSScriptRoot $relative
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { Fail "Missing checked file: $relative" }
        $actual = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { Fail "Checksum mismatch: $relative" }
    }
}

function Test-Bundle {
    Invoke-NativeChecked 'docker' @('info')
    Invoke-NativeChecked 'docker' @('compose', 'version')
    Test-Environment
    Test-Checksums
    $composeText = Get-Content -LiteralPath $ComposeFile -Raw
    if ($composeText -notmatch 'ghcr\.io/jarxunlai/muriarc-server-tester@sha256:[0-9a-f]{64}') {
        Fail 'Server image is not pinned to the expected GHCR digest.'
    }
    if ($composeText -notmatch 'postgres:17-bookworm@sha256:[0-9a-f]{64}') {
        Fail 'PostgreSQL image is not digest pinned.'
    }
    if ($composeText -match ':latest|5432:5432|/var/run/docker\.sock|0\.0\.0\.0:[^ ]*:8787') {
        Fail 'Compose policy violation.'
    }
    Invoke-Compose @('config', '--quiet')
    $images = & docker compose --env-file $EnvFile --project-name $Project --file $ComposeFile config --images
    if ($LASTEXITCODE -ne 0) { Fail 'Could not resolve Compose images.' }
    foreach ($image in @($images | Sort-Object -Unique)) {
        Invoke-NativeChecked 'docker' @('buildx', 'imagetools', 'inspect', $image)
    }
    Write-Host 'PASS: bundle, environment, Compose policy and pinned images verified.'
}

function Initialize-Empty {
    Test-Environment
    if ($Mode -ne 'empty') { Fail 'init-empty requires the empty template.' }
    Assert-Fresh
    try {
        Invoke-ComposeBootstrap @('up', '--detach', '--wait', '--wait-timeout', '240', 'db', 'server')
        Wait-Ready
        Invoke-ComposeBootstrap @('stop', 'server')
        Invoke-Compose @('up', '--detach', '--wait', '--wait-timeout', '240', 'server')
        Wait-Ready
        Write-Host 'Empty Tester initialized. Bootstrap is now disabled; volumes are retained.'
    } catch {
        Write-Warning 'Initialization failed; containers and volumes are preserved. Do not clear or SQL-patch them.'
        throw
    }
}

function Initialize-Demo {
    Test-Environment
    if ($Mode -ne 'demo') { Fail 'init-demo requires the demo template.' }
    if ((Get-DotEnvValue 'MURIARC_LAB_ID') -ne '4d555249-4152-4300-0000-000000000001') { Fail 'Demo Lab UUID must remain fixed.' }
    if ((Get-DotEnvValue 'MURIARC_ROOT_USER_ID') -ne '4d555249-4152-4300-0000-000000000002') { Fail 'Demo Root user UUID must remain fixed.' }
    Assert-Fresh
    try {
        Invoke-Compose @('up', '--detach', '--wait', '--wait-timeout', '180', 'db')
        Invoke-Compose @('--profile', 'demo-tools', 'run', '--rm', 'seed-standard-v1')
        Invoke-Compose @('--profile', 'demo-tools', 'run', '--rm', 'seed-standard-v1', 'verify-postgres',
            '--fixture', '/opt/muriarc/fixtures/standard-v1', '--output', '/var/lib/muriarc/generation',
            '--source-commit', $SourceCommit)
        Invoke-Compose @('--profile', 'demo-tools', 'run', '--rm', '--entrypoint', '/bin/sh', 'seed-standard-v1',
            '-ec', 'install -m 0600 /var/lib/muriarc/generation/deployment-generation.json /var/lib/muriarc/generation/data/deployment-generation.json')
        Invoke-Compose @('up', '--detach', '--wait', '--wait-timeout', '240', 'server')
        Wait-Ready
        Write-Host 'Synthetic standard-v1 Tester initialized and verified. Bootstrap remains disabled.'
    } catch {
        Write-Warning 'Demo initialization failed; containers and volumes are preserved. Do not clear or SQL-patch them.'
        throw
    }
}

switch ($Command) {
    'verify' { Test-Bundle }
    'init-empty' { Initialize-Empty }
    'init-demo' { Initialize-Demo }
    'up' { Test-Environment; Assert-Initialized; Invoke-Compose @('up', '--detach', '--wait', '--wait-timeout', '240', 'db', 'server'); Wait-Ready }
    'status' { Test-Environment; Invoke-Compose @('ps'); Wait-Ready }
    'logs' { Test-Environment; Invoke-Compose @('logs', '--no-color', '--tail', '200', 'db', 'server') }
    'down' { Test-Environment; Invoke-Compose @('down', '--remove-orphans'); Write-Host 'Stopped; named volumes were preserved.' }
}
