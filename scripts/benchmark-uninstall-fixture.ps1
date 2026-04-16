param(
    [string]$Name = 'uninstall-fixture',
    [string]$UpstreamScoopRoot,
    [int]$Warmup = 1,
    [int]$Runs = 3
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'root-helpers.ps1')
$upstreamRoot = Resolve-UpstreamAppRoot -ScoopRoot $UpstreamScoopRoot
$rustExe = Join-Path $repoRoot 'target\release\scoop.exe'
$resultsDir = Join-Path $repoRoot 'benchmarks'
$wrapperDir = Join-Path $resultsDir 'wrappers'
$fixtureRoot = Join-Path $env:TEMP ('scoop-rs-uninstall-bench-' + [guid]::NewGuid())
$localRoot = Join-Path $fixtureRoot 'local'
$globalRoot = Join-Path $fixtureRoot 'global'
$payloadRoot = Join-Path $fixtureRoot 'payload'
$configHome = Join-Path $fixtureRoot 'config'

function Quote-CmdArg([string]$Value) {
    '"' + $Value.Replace('"', '""') + '"'
}

function Join-CmdCommand([string[]]$Parts) {
    ($Parts | ForEach-Object { Quote-CmdArg $_ }) -join ' '
}

function New-BenchmarkWrapper(
    [string]$Path,
    [string[]]$CommandParts
) {
    $command = Join-CmdCommand $CommandParts
    $body = @(
        '@echo off',
        'setlocal',
        ('set "SCOOP=' + $localRoot + '"'),
        ('set "SCOOP_GLOBAL=' + $globalRoot + '"'),
        ('set "XDG_CONFIG_HOME=' + $configHome + '"'),
        "$command >nul",
        'exit /b %errorlevel%'
    )
    Set-Content -Path $Path -Value ($body -join "`r`n") -NoNewline -Encoding Ascii
}

function Initialize-Fixture {
    New-Item -ItemType Directory -Path (Join-Path $localRoot 'buckets\main\bucket') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $localRoot 'apps') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $localRoot 'shims') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $globalRoot 'apps') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $globalRoot 'shims') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $configHome 'scoop') -Force | Out-Null
    New-Item -ItemType Directory -Path $payloadRoot -Force | Out-Null
    Set-Content -Path (Join-Path $configHome 'scoop\config.json') -Value '{}' -Encoding Ascii

    $payloadDir = Join-Path $payloadRoot 'demo-payload'
    New-Item -ItemType Directory -Path $payloadDir -Force | Out-Null
    Set-Content -Path (Join-Path $payloadDir 'demo.exe') -Value 'demo' -Encoding Ascii

    $archive = Join-Path $payloadRoot 'demo.zip'
    Compress-Archive -Path (Join-Path $payloadDir '*') -DestinationPath $archive -Force
    $hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower()
    @{ version = '1.2.3'; url = $archive; hash = $hash; bin = 'demo.exe' } |
        ConvertTo-Json -Compress |
        Set-Content -Path (Join-Path $localRoot 'buckets\main\bucket\demo.json') -Encoding Ascii
}

function New-PrepareScript([string]$BenchmarkName) {
    $path = Join-Path $resultsDir "$BenchmarkName-prepare.ps1"
    $upstreamInstall = Join-Path $upstreamRoot 'bin\scoop.ps1'
    $body = @(
        '$ErrorActionPreference = ''Stop'''
        ('$localRoot = ' + (Quote-CmdArg $localRoot))
        ('$globalRoot = ' + (Quote-CmdArg $globalRoot))
        ('$configHome = ' + (Quote-CmdArg $configHome))
        ('$upstreamInstall = ' + (Quote-CmdArg $upstreamInstall))
        '$resetTargets = @('
        '    (Join-Path $localRoot ''apps''),'
        '    (Join-Path $localRoot ''shims''),'
        '    (Join-Path $localRoot ''cache''),'
        '    (Join-Path $globalRoot ''apps''),'
        '    (Join-Path $globalRoot ''shims''),'
        '    (Join-Path $globalRoot ''cache'')'
        ')'
        'foreach ($target in $resetTargets) {'
        '    if (Test-Path -LiteralPath $target) {'
        '        cmd /c rd /s /q "$target" | Out-Null'
        '    }'
        '}'
        'New-Item -ItemType Directory -Path (Join-Path $localRoot ''apps'') -Force | Out-Null'
        'New-Item -ItemType Directory -Path (Join-Path $localRoot ''shims'') -Force | Out-Null'
        'New-Item -ItemType Directory -Path (Join-Path $globalRoot ''apps'') -Force | Out-Null'
        'New-Item -ItemType Directory -Path (Join-Path $globalRoot ''shims'') -Force | Out-Null'
        '$env:SCOOP = $localRoot'
        '$env:SCOOP_GLOBAL = $globalRoot'
        '$env:XDG_CONFIG_HOME = $configHome'
        '& pwsh -NoProfile -ExecutionPolicy Bypass -File $upstreamInstall install demo --no-update-scoop *> $null'
        'if ($LASTEXITCODE -ne 0) {'
        '    throw ''fixture install failed'''
        '}'
    )
    Set-Content -Path $path -Value ($body -join "`r`n") -NoNewline -Encoding Ascii
    return $path
}

cargo build --release --bin scoop | Out-Null
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
New-Item -ItemType Directory -Force -Path $wrapperDir | Out-Null
Initialize-Fixture

$jsonPath = Join-Path $resultsDir "$Name.json"
$prepareScript = New-PrepareScript $Name
$upstreamWrapper = Join-Path $wrapperDir "$Name-upstream.cmd"
$rustWrapper = Join-Path $wrapperDir "$Name-rust.cmd"

New-BenchmarkWrapper -Path $upstreamWrapper -CommandParts @(
    'pwsh',
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    (Join-Path $upstreamRoot 'bin\scoop.ps1'),
    'uninstall',
    'demo'
)
New-BenchmarkWrapper -Path $rustWrapper -CommandParts @(
    $rustExe,
    'uninstall',
    'demo'
)

try {
    hyperfine `
        --warmup $Warmup `
        --runs $Runs `
        --prepare ('pwsh -NoProfile -ExecutionPolicy Bypass -File ' + (Quote-CmdArg $prepareScript)) `
        --export-json $jsonPath `
        --command-name "upstream:$Name" ("cmd.exe /c call " + (Quote-CmdArg $upstreamWrapper)) `
        --command-name "scoop-rs:$Name" ("cmd.exe /c call " + (Quote-CmdArg $rustWrapper))
}
finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        cmd /c rd /s /q "$fixtureRoot" | Out-Null
    }
}
