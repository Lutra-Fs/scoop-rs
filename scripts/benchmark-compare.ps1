param(
    [Parameter(Mandatory)]
    [string]$Name,

    [Parameter(Mandatory)]
    [string[]]$Args,

    [string]$UpstreamScoopRoot,
    [string]$SfsuCommand = 'sfsu',
    [string]$HokCommand = 'hok',
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

function Resolve-CommandParts(
    [string]$CommandName,
    [string]$DisplayName
) {
    $command = Get-Command -Name $CommandName -CommandType Application,ExternalScript -ErrorAction Stop | Select-Object -First 1
    $path = $command.Path

    if ([string]::IsNullOrWhiteSpace($path)) {
        throw "Unable to resolve $DisplayName command path for '$CommandName'. Install it with Scoop and make sure it is on PATH."
    }

    if ([System.IO.Path]::GetExtension($path).ToLowerInvariant() -eq '.ps1') {
        return @(
            'pwsh',
            '-NoProfile',
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            $path
        )
    }

    return @($path)
}

cargo build --release --bin scoop
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
New-Item -ItemType Directory -Force -Path $wrapperDir | Out-Null

$jsonPath = Join-Path $resultsDir "$Name.json"
$commandSets = @(
    [pscustomobject]@{
        Label = "upstream:$Name"
        Parts = @(
            'pwsh',
            '-NoProfile',
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            (Join-Path $upstreamRoot 'bin\scoop.ps1')
        ) + $Args
    },
    [pscustomobject]@{
        Label = "scoop-rs:$Name"
        Parts = @($rustExe) + $Args
    },
    [pscustomobject]@{
        Label = "sfsu:$Name"
        Parts = (Resolve-CommandParts -CommandName $SfsuCommand -DisplayName 'sfsu') + $Args
    },
    [pscustomobject]@{
        Label = "hok:$Name"
        Parts = (Resolve-CommandParts -CommandName $HokCommand -DisplayName 'hok') + $Args
    }
)

$hyperfineArgs = @(
    '--warmup', $Warmup,
    '--runs', $Runs,
    '--export-json', $jsonPath
)

foreach ($commandSet in $commandSets) {
    $wrapperPath = Join-Path $wrapperDir ($commandSet.Label.Replace(':', '-') + '.cmd')
    New-BenchmarkWrapper -Path $wrapperPath -CommandParts $commandSet.Parts -Iterations $IterationsPerRun
    $hyperfineArgs += @(
        '--command-name', $commandSet.Label,
        'cmd.exe /c call ' + (Quote-CmdArg $wrapperPath)
    )
}

hyperfine @hyperfineArgs
