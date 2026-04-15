param(
    [ValidateSet('cold-build', 'rebuild', 'all')]
    [string]$Scenario = 'all',

    [string]$Query = 'google',

    [int]$Warmup = 1,
    [int]$Runs = 5,

    [string]$ScoopRoot = 'D:\Applications\Scoop'
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$rustExe = Join-Path $repoRoot 'target\release\scoop.exe'
$resultsDir = Join-Path $repoRoot 'benchmarks'
$dbPath = Join-Path $ScoopRoot 'scoop.db'
$tempConfigHome = Join-Path $env:TEMP ('scoop-rs-search-cache-' + [guid]::NewGuid())
$backupPath = Join-Path $env:TEMP ('scoop-rs-search-cache-backup-' + [guid]::NewGuid() + '.db')
$configDir = Join-Path $tempConfigHome 'scoop'

function Quote-CmdArg([string]$Value) {
    return '"' + $Value.Replace('"', '""') + '"'
}

function Write-Config {
    New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    Set-Content -Path (Join-Path $configDir 'config.json') -Value '{"use_sqlite_cache":true}' -Encoding Ascii
}

function Invoke-SearchQuiet {
    $previousConfig = [Environment]::GetEnvironmentVariable('XDG_CONFIG_HOME', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('XDG_CONFIG_HOME', $tempConfigHome, 'Process')
        & $rustExe search $Query *> $null
    } finally {
        [Environment]::SetEnvironmentVariable('XDG_CONFIG_HOME', $previousConfig, 'Process')
    }
}

function New-PrepareScript([string]$Name, [string[]]$BodyLines) {
    $path = Join-Path $resultsDir "prepare-$Name.ps1"
    Set-Content -Path $path -Value ($BodyLines -join "`r`n") -NoNewline -Encoding Ascii
    return $path
}

function Invoke-HyperfineScenario([string]$Name, [string]$PrepareScript) {
    $jsonPath = Join-Path $resultsDir "$Name.json"
    $command = '$env:XDG_CONFIG_HOME=' + (Quote-CmdArg $tempConfigHome) + '; & ' + (Quote-CmdArg $rustExe) + ' search ' + (Quote-CmdArg $Query) + ' *> $null'
    hyperfine `
        --warmup $Warmup `
        --runs $Runs `
        --prepare ('pwsh -NoProfile -ExecutionPolicy Bypass -File ' + (Quote-CmdArg $PrepareScript)) `
        --export-json $jsonPath `
        --command-name "scoop-rs:$Name" `
        ('pwsh -NoProfile -ExecutionPolicy Bypass -Command ' + (Quote-CmdArg $command))
}

cargo build --release --bin scoop | Out-Null
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
Write-Config

$hadOriginalDb = Test-Path -LiteralPath $dbPath
if ($hadOriginalDb) {
    Move-Item -LiteralPath $dbPath -Destination $backupPath -Force
}

try {
    $coldPrepare = New-PrepareScript -Name 'search-cache-cold-build' -BodyLines @(
        '$ErrorActionPreference = ''Stop'''
        ('$dbPath = ' + (Quote-CmdArg $dbPath))
        'if (Test-Path -LiteralPath $dbPath) { Remove-Item -LiteralPath $dbPath -Force }'
    )
    $rebuildPrepare = New-PrepareScript -Name 'search-cache-rebuild' -BodyLines @(
        '$ErrorActionPreference = ''Stop'''
        ('$dbPath = ' + (Quote-CmdArg $dbPath))
        ('$dbDir = Split-Path -Parent $dbPath')
        'New-Item -ItemType Directory -Path $dbDir -Force | Out-Null'
        '[System.IO.File]::WriteAllText($dbPath, ''not a sqlite database'')'
    )

    switch ($Scenario) {
        'cold-build' {
            Invoke-HyperfineScenario -Name 'search-cache-cold-build' -PrepareScript $coldPrepare
        }
        'rebuild' {
            Invoke-HyperfineScenario -Name 'search-cache-rebuild' -PrepareScript $rebuildPrepare
        }
        'all' {
            Invoke-HyperfineScenario -Name 'search-cache-cold-build' -PrepareScript $coldPrepare
            Invoke-HyperfineScenario -Name 'search-cache-rebuild' -PrepareScript $rebuildPrepare
        }
    }
}
finally {
    if (Test-Path -LiteralPath $dbPath) {
        Remove-Item -LiteralPath $dbPath -Force
    }
    if ($hadOriginalDb -and (Test-Path -LiteralPath $backupPath)) {
        Move-Item -LiteralPath $backupPath -Destination $dbPath -Force
    }
    if (Test-Path -LiteralPath $tempConfigHome) {
        Remove-Item -LiteralPath $tempConfigHome -Recurse -Force
    }
}
