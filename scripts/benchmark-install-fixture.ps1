param(
    [string]$Name = 'install-fixture',
    [int]$Warmup = 1,
    [int]$Runs = 3
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$upstreamRoot = 'D:\Applications\Scoop\apps\scoop\current'
$rustExe = Join-Path $repoRoot 'target\release\scoop.exe'
$resultsDir = Join-Path $repoRoot 'benchmarks'
$wrapperDir = Join-Path $resultsDir 'wrappers'
$fixtureRoot = Join-Path $env:TEMP ('scoop-rs-install-bench-' + [guid]::NewGuid())
$localRoot = Join-Path $fixtureRoot 'local'
$globalRoot = Join-Path $fixtureRoot 'global'
$payloadRoot = Join-Path $fixtureRoot 'payload'
$configHome = Join-Path $fixtureRoot 'config'

function Quote-CmdArg([string]$Value) {
    return '"' + $Value.Replace('"', '""') + '"'
}

function Join-CmdCommand([string[]]$Parts) {
    $quoted = $Parts | ForEach-Object { Quote-CmdArg $_ }
    return $quoted -join ' '
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

    $payloadFile = Join-Path $payloadRoot 'README.txt'
    $archive = Join-Path $payloadRoot 'demo.zip'
    Set-Content -Path $payloadFile -Value 'demo' -Encoding Ascii
    Compress-Archive -Path $payloadFile -DestinationPath $archive -Force
    $hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower()
    @{ version = '1.2.3'; url = $archive; hash = $hash } |
        ConvertTo-Json -Compress |
        Set-Content -Path (Join-Path $localRoot 'buckets\main\bucket\demo.json') -Encoding Ascii
}

function New-PrepareScript([string]$Name) {
    $path = Join-Path $resultsDir "$Name-prepare.ps1"
    $body = @(
        '$ErrorActionPreference = ''Stop'''
        ('$localRoot = ' + (Quote-CmdArg $localRoot))
        ('$globalRoot = ' + (Quote-CmdArg $globalRoot))
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
    'install',
    'demo',
    '--no-update-scoop'
)
New-BenchmarkWrapper -Path $rustWrapper -CommandParts @(
    $rustExe,
    'install',
    'demo',
    '--no-update-scoop'
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
