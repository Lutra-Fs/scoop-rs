param(
    [ValidateSet('all', 'list', 'search-cold', 'search-warm', 'info-full', 'info-fair')]
    [string]$Scenario = 'all',

    [string]$UpstreamScoopRoot,
    [int]$IterationsPerRun = 50,
    [int]$Warmup = 5,
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

function Invoke-HyperfineBatch(
    [string]$Name,
    [object[]]$CommandSets
) {
    cargo build --release --bin scoop
    New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
    New-Item -ItemType Directory -Force -Path $wrapperDir | Out-Null

    $jsonPath = Join-Path $resultsDir "$Name.json"
    $hyperfineArgs = @(
        '--warmup', $Warmup,
        '--runs', $Runs,
        '--export-json', $jsonPath
    )

    foreach ($commandSet in $CommandSets) {
        $wrapperPath = Join-Path $wrapperDir ($commandSet.Label.Replace(':', '-') + '.cmd')
        New-BenchmarkWrapper -Path $wrapperPath -CommandParts $commandSet.Parts -Iterations $IterationsPerRun
        $hyperfineArgs += @(
            '--command-name', $commandSet.Label,
            'cmd.exe /c call ' + (Quote-CmdArg $wrapperPath)
        )
    }

    hyperfine @hyperfineArgs
}

function New-CommandSet([string]$Label, [string[]]$Parts) {
    [pscustomobject]@{
        Label = $Label
        Parts = $Parts
    }
}

$upstreamBase = @(
    'pwsh',
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    (Join-Path $upstreamRoot 'bin\scoop.ps1')
)

$sfsuBase = Resolve-CommandParts -CommandName 'sfsu' -DisplayName 'sfsu'
$hokBase = Resolve-CommandParts -CommandName 'hok' -DisplayName 'hok'

switch ($Scenario) {
    'list' {
        Invoke-HyperfineBatch -Name 'list-fourway' -CommandSets @(
            (New-CommandSet 'upstream:list' ($upstreamBase + @('list'))),
            (New-CommandSet 'scoop-rs:list' (@($rustExe) + @('list'))),
            (New-CommandSet 'sfsu:list' ($sfsuBase + @('list'))),
            (New-CommandSet 'hok:list' ($hokBase + @('list')))
        )
    }
    'search-cold' {
        Invoke-HyperfineBatch -Name 'search-cold-fourway' -CommandSets @(
            (New-CommandSet 'upstream:search' ($upstreamBase + @('search', 'google'))),
            (New-CommandSet 'scoop-rs:search' (@($rustExe) + @('search', 'google'))),
            (New-CommandSet 'sfsu:search' ($sfsuBase + @('search', 'google'))),
            (New-CommandSet 'hok:search' ($hokBase + @('search', 'google')))
        )
    }
    'search-warm' {
        Invoke-HyperfineBatch -Name 'search-warm-two-way' -CommandSets @(
            (New-CommandSet 'upstream:search' ($upstreamBase + @('search', 'google'))),
            (New-CommandSet 'scoop-rs:search' (@($rustExe) + @('search', 'google')))
        )
    }
    'info-full' {
        Invoke-HyperfineBatch -Name 'info-full-fourway' -CommandSets @(
            (New-CommandSet 'upstream:info' ($upstreamBase + @('info', 'sfsu'))),
            (New-CommandSet 'scoop-rs:info' (@($rustExe) + @('info', 'sfsu'))),
            (New-CommandSet 'sfsu:info' ($sfsuBase + @('info', 'sfsu'))),
            (New-CommandSet 'hok:info' ($hokBase + @('info', 'sfsu')))
        )
    }
    'info-fair' {
        Invoke-HyperfineBatch -Name 'info-fair-sfsu-hok' -CommandSets @(
            (New-CommandSet 'sfsu:info' ($sfsuBase + @('info', 'sfsu', '--disable-updated'))),
            (New-CommandSet 'hok:info' ($hokBase + @('info', 'sfsu')))
        )
    }
    'all' {
        & $PSCommandPath -Scenario list -UpstreamScoopRoot $UpstreamScoopRoot -IterationsPerRun $IterationsPerRun -Warmup $Warmup -Runs $Runs
        & $PSCommandPath -Scenario search-cold -UpstreamScoopRoot $UpstreamScoopRoot -IterationsPerRun $IterationsPerRun -Warmup $Warmup -Runs $Runs
        & $PSCommandPath -Scenario search-warm -UpstreamScoopRoot $UpstreamScoopRoot -IterationsPerRun $IterationsPerRun -Warmup $Warmup -Runs $Runs
        & $PSCommandPath -Scenario info-full -UpstreamScoopRoot $UpstreamScoopRoot -IterationsPerRun $IterationsPerRun -Warmup $Warmup -Runs $Runs
        & $PSCommandPath -Scenario info-fair -UpstreamScoopRoot $UpstreamScoopRoot -IterationsPerRun $IterationsPerRun -Warmup $Warmup -Runs $Runs
    }
}
