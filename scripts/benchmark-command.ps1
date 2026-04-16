param(
    [Parameter(Mandatory)]
    [string]$Name,

    [Parameter(Mandatory)]
    [string[]]$Args,

    [string]$UpstreamScoopRoot,
    [int]$IterationsPerRun = 50,
    [int]$Warmup = 3,
    [int]$Runs = 20
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'root-helpers.ps1')
$upstreamRoot = Resolve-UpstreamAppRoot -ScoopRoot $UpstreamScoopRoot
$rustExe = Join-Path $repoRoot 'target\release\scoop.exe'
$resultsDir = Join-Path $repoRoot 'benchmarks'
$wrapperDir = Join-Path $resultsDir 'wrappers'

function Quote-CmdArg([string]$Value) {
    return '"' + $Value.Replace('"', '""') + '"'
}

function Join-CmdCommand([string[]]$Parts) {
    $quoted = $Parts | ForEach-Object { Quote-CmdArg $_ }
    return $quoted -join ' '
}

function New-BenchmarkWrapper(
    [string]$Path,
    [string[]]$CommandParts,
    [int]$Iterations
) {
    $command = Join-CmdCommand $CommandParts
    $body = @(
        '@echo off',
        'setlocal'
    )

    if ($Iterations -le 1) {
        $body += "$command >nul"
    } else {
        $body += "for /L %%i in (1,1,$Iterations) do $command >nul"
    }

    $body += 'exit /b %errorlevel%'
    Set-Content -Path $Path -Value ($body -join "`r`n") -NoNewline -Encoding Ascii
}

cargo build --release --bin scoop
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
New-Item -ItemType Directory -Force -Path $wrapperDir | Out-Null

$jsonPath = Join-Path $resultsDir "$Name.json"
$upstreamWrapper = Join-Path $wrapperDir "$Name-upstream.cmd"
$rustWrapper = Join-Path $wrapperDir "$Name-rust.cmd"
$upstreamCommandParts = @(
    'pwsh',
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    (Join-Path $upstreamRoot 'bin\scoop.ps1')
) + $Args
$rustCommandParts = @($rustExe) + $Args

New-BenchmarkWrapper -Path $upstreamWrapper -CommandParts $upstreamCommandParts -Iterations $IterationsPerRun
New-BenchmarkWrapper -Path $rustWrapper -CommandParts $rustCommandParts -Iterations $IterationsPerRun

hyperfine `
    --warmup $Warmup `
    --runs $Runs `
    --export-json $jsonPath `
    --command-name "upstream:$Name" ("cmd.exe /c call " + (Quote-CmdArg $upstreamWrapper)) `
    --command-name "scoop-rs:$Name" ("cmd.exe /c call " + (Quote-CmdArg $rustWrapper))
