param(
    [string]$Name = 'update-fixture',
    [int]$Warmup = 1,
    [int]$Runs = 3
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$upstreamTemplate = 'D:\Applications\Scoop\apps\scoop\current'
$rustExe = Join-Path $repoRoot 'target\release\scoop.exe'
$resultsDir = Join-Path $repoRoot 'benchmarks'
$wrapperDir = Join-Path $resultsDir 'wrappers'
$fixtureRoot = Join-Path $env:TEMP ('scoop-rs-update-bench-' + [guid]::NewGuid())
$upstreamRoot = Join-Path $fixtureRoot 'upstream-local'
$rustRoot = Join-Path $fixtureRoot 'rust-local'
$globalRoot = Join-Path $fixtureRoot 'global'
$upstreamConfigHome = Join-Path $fixtureRoot 'upstream-config'
$rustConfigHome = Join-Path $fixtureRoot 'rust-config'
$gitRoot = Join-Path $fixtureRoot 'git-fixtures'
$payloadRoot = Join-Path $fixtureRoot 'payloads'

function Quote-CmdArg([string]$Value) {
    '"' + $Value.Replace('"', '""') + '"'
}

function Join-CmdCommand([string[]]$Parts) {
    ($Parts | ForEach-Object { Quote-CmdArg $_ }) -join ' '
}

function New-BenchmarkWrapper(
    [string]$Path,
    [string]$LocalRoot,
    [string]$ConfigHome,
    [string[]]$CommandParts
) {
    $command = Join-CmdCommand $CommandParts
    $body = @(
        '@echo off',
        'setlocal',
        ('set "SCOOP=' + $LocalRoot + '"'),
        ('set "SCOOP_GLOBAL=' + $globalRoot + '"'),
        ('set "XDG_CONFIG_HOME=' + $ConfigHome + '"'),
        "$command >nul",
        'exit /b %errorlevel%'
    )
    Set-Content -Path $Path -Value ($body -join "`r`n") -NoNewline -Encoding Ascii
}

function Run-Git {
    param(
        [string]$WorkingDirectory,
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Args
    )
    $output = & git -C $WorkingDirectory @Args 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Args -join ' ') failed in $WorkingDirectory`n$output"
    }
    return ($output -join "`n").Trim()
}

function New-BareRepoFromDirectory(
    [string]$TemplatePath,
    [string]$Name,
    [string]$TargetPath,
    [string]$MutationRelativePath,
    [string]$MutationContent
) {
    $source = Join-Path $gitRoot "$Name-src"
    $remote = Join-Path $gitRoot "$Name-remote.git"
    New-Item -ItemType Directory -Path $source -Force | Out-Null
    & robocopy $TemplatePath $source /E /XD .git target .codex .jj > $null
    if ($LASTEXITCODE -ge 8) {
        throw "robocopy $TemplatePath $source failed with exit code $LASTEXITCODE"
    }

    & git init $source > $null
    if ($LASTEXITCODE -ne 0) {
        throw "git init $source failed"
    }
    Run-Git $source config user.name Codex
    Run-Git $source config user.email codex@example.invalid
    Run-Git $source config commit.gpgsign false
    Run-Git $source add .
    Run-Git $source commit -m seed
    & git clone --bare $source $remote > $null
    if ($LASTEXITCODE -ne 0) {
        throw "git clone --bare $source $remote failed"
    }
    Run-Git $source remote add origin $remote

    $targetParent = Split-Path -Parent $TargetPath
    New-Item -ItemType Directory -Path $targetParent -Force | Out-Null
    & git clone -q $remote $TargetPath
    if ($LASTEXITCODE -ne 0) {
        throw "git clone $remote $TargetPath failed"
    }
    $initialHead = Run-Git $TargetPath rev-parse HEAD
    $branch = Run-Git $TargetPath branch --show-current

    $mutationPath = Join-Path $source $MutationRelativePath
    New-Item -ItemType Directory -Path (Split-Path -Parent $mutationPath) -Force | Out-Null
    Set-Content -Path $mutationPath -Value $MutationContent -Encoding Ascii
    Run-Git $source add .
    Run-Git $source commit -m update
    Run-Git $source push origin $branch

    return @{
        InitialHead = $initialHead
        Target = $TargetPath
    }
}

function New-BucketFixture(
    [string]$Root,
    [string]$Name,
    [string]$InitialManifest,
    [string]$UpdatedManifest
) {
    $source = Join-Path $gitRoot "$Name-src"
    $remote = Join-Path $gitRoot "$Name-remote.git"
    $target = Join-Path $Root 'buckets\main'
    New-Item -ItemType Directory -Path $source -Force | Out-Null
    & git init $source > $null
    if ($LASTEXITCODE -ne 0) {
        throw "git init $source failed"
    }
    Run-Git $source config user.name Codex
    Run-Git $source config user.email codex@example.invalid
    Run-Git $source config commit.gpgsign false
    New-Item -ItemType Directory -Path (Join-Path $source 'bucket') -Force | Out-Null
    Set-Content -Path (Join-Path $source 'bucket\scoop.json') -Value $InitialManifest -Encoding Ascii
    Run-Git $source add .
    Run-Git $source commit -m seed
    & git clone --bare $source $remote > $null
    if ($LASTEXITCODE -ne 0) {
        throw "git clone --bare $source $remote failed"
    }
    Run-Git $source remote add origin $remote
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    & git clone -q $remote $target
    if ($LASTEXITCODE -ne 0) {
        throw "git clone $remote $target failed"
    }
    $initialHead = Run-Git $target rev-parse HEAD
    $branch = Run-Git $target branch --show-current
    Set-Content -Path (Join-Path $source 'bucket\scoop.json') -Value $UpdatedManifest -Encoding Ascii
    Run-Git $source add .
    Run-Git $source commit -m update
    Run-Git $source push origin $branch
    return @{
        InitialHead = $initialHead
        Target = $target
    }
}

function Initialize-UpstreamFixture {
    $core = New-BareRepoFromDirectory `
        -TemplatePath $upstreamTemplate `
        -Name 'upstream-core' `
        -TargetPath (Join-Path $upstreamRoot 'apps\scoop\current') `
        -MutationRelativePath 'README.md' `
        -MutationContent 'bench update'
    $bucket = New-BucketFixture `
        -Root $upstreamRoot `
        -Name 'upstream-bucket' `
        -InitialManifest '{"version":"1.0.0"}' `
        -UpdatedManifest '{"version":"2.0.0"}'
    return @{
        CoreHead = $core.InitialHead
        BucketHead = $bucket.InitialHead
    }
}

function Seed-RustInstalledScoop([string]$BinaryContent) {
    $versionDir = Join-Path $rustRoot 'apps\scoop\1.0.0'
    $currentDir = Join-Path $rustRoot 'apps\scoop\current'
    New-Item -ItemType Directory -Path $versionDir -Force | Out-Null
    New-Item -ItemType Directory -Path $currentDir -Force | Out-Null
    Set-Content -Path (Join-Path $versionDir 'install.json') -Value '{"bucket":"main","architecture":"64bit"}' -Encoding Ascii
    Set-Content -Path (Join-Path $versionDir 'manifest.json') -Value '{"version":"1.0.0"}' -Encoding Ascii
    Set-Content -Path (Join-Path $versionDir 'scoop.exe') -Value $BinaryContent -Encoding Ascii
    Set-Content -Path (Join-Path $currentDir 'manifest.json') -Value '{"version":"1.0.0"}' -Encoding Ascii
}

function Initialize-RustFixture {
    New-Item -ItemType Directory -Path $payloadRoot -Force | Out-Null
    $payloadFile = Join-Path $payloadRoot 'scoop.exe'
    $archive = Join-Path $payloadRoot 'scoop.zip'
    Set-Content -Path $payloadFile -Value 'new binary' -Encoding Ascii
    Compress-Archive -Path $payloadFile -DestinationPath $archive -Force
    $hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower()
    $updatedManifest = (@{
        version = '2.0.0'
        url = $archive
        hash = $hash
        bin = 'scoop.exe'
    } | ConvertTo-Json -Compress)
    $bucket = New-BucketFixture `
        -Root $rustRoot `
        -Name 'rust-bucket' `
        -InitialManifest '{"version":"1.0.0"}' `
        -UpdatedManifest $updatedManifest
    Seed-RustInstalledScoop 'old binary'
    return @{
        BucketHead = $bucket.InitialHead
    }
}

function Initialize-Fixture {
    New-Item -ItemType Directory -Path $upstreamRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $rustRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $globalRoot -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $upstreamConfigHome 'scoop') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $rustConfigHome 'scoop') -Force | Out-Null
    New-Item -ItemType Directory -Path $gitRoot -Force | Out-Null
    Set-Content -Path (Join-Path $upstreamConfigHome 'scoop\config.json') -Value '{}' -Encoding Ascii
    Set-Content -Path (Join-Path $rustConfigHome 'scoop\config.json') -Value '{}' -Encoding Ascii
    $upstreamState = Initialize-UpstreamFixture
    $rustState = Initialize-RustFixture
    return @{
        UpstreamCoreHead = $upstreamState.CoreHead
        UpstreamBucketHead = $upstreamState.BucketHead
        RustBucketHead = $rustState.BucketHead
    }
}

function New-PrepareScript([string]$Path, [hashtable]$State) {
    $body = @(
        '$ErrorActionPreference = ''Stop'''
        ('$upstreamRoot = ' + (Quote-CmdArg $upstreamRoot))
        ('$rustRoot = ' + (Quote-CmdArg $rustRoot))
        ('$globalRoot = ' + (Quote-CmdArg $globalRoot))
        ('$upstreamConfigHome = ' + (Quote-CmdArg $upstreamConfigHome))
        ('$rustConfigHome = ' + (Quote-CmdArg $rustConfigHome))
        ('$upstreamCoreHead = ' + (Quote-CmdArg $State.UpstreamCoreHead))
        ('$upstreamBucketHead = ' + (Quote-CmdArg $State.UpstreamBucketHead))
        ('$rustBucketHead = ' + (Quote-CmdArg $State.RustBucketHead))
        '& git -C (Join-Path $upstreamRoot ''apps\scoop\current'') reset --hard $upstreamCoreHead -q | Out-Null'
        '& git -C (Join-Path $upstreamRoot ''apps\scoop\current'') clean -fdqx | Out-Null'
        '& git -C (Join-Path $upstreamRoot ''buckets\main'') reset --hard $upstreamBucketHead -q | Out-Null'
        '& git -C (Join-Path $upstreamRoot ''buckets\main'') clean -fdqx | Out-Null'
        '& git -C (Join-Path $rustRoot ''buckets\main'') reset --hard $rustBucketHead -q | Out-Null'
        '& git -C (Join-Path $rustRoot ''buckets\main'') clean -fdqx | Out-Null'
        '$rustApps = Join-Path $rustRoot ''apps'''
        'if (Test-Path -LiteralPath $rustApps) { cmd /c rd /s /q "$rustApps" | Out-Null }'
        'New-Item -ItemType Directory -Path (Join-Path $rustRoot ''apps\scoop\1.0.0'') -Force | Out-Null'
        'New-Item -ItemType Directory -Path (Join-Path $rustRoot ''apps\scoop\current'') -Force | Out-Null'
        'Set-Content -Path (Join-Path $rustRoot ''apps\scoop\1.0.0\install.json'') -Value ''{"bucket":"main","architecture":"64bit"}'' -Encoding Ascii'
        'Set-Content -Path (Join-Path $rustRoot ''apps\scoop\1.0.0\manifest.json'') -Value ''{"version":"1.0.0"}'' -Encoding Ascii'
        'Set-Content -Path (Join-Path $rustRoot ''apps\scoop\1.0.0\scoop.exe'') -Value ''old binary'' -Encoding Ascii'
        'Set-Content -Path (Join-Path $rustRoot ''apps\scoop\current\manifest.json'') -Value ''{"version":"1.0.0"}'' -Encoding Ascii'
        'foreach ($root in @($upstreamRoot, $rustRoot, $globalRoot)) {'
        '    $shims = Join-Path $root ''shims'''
        '    if (Test-Path -LiteralPath $shims) { cmd /c rd /s /q "$shims" | Out-Null }'
        '    New-Item -ItemType Directory -Path $shims -Force | Out-Null'
        '}'
        'New-Item -ItemType Directory -Path (Join-Path $upstreamConfigHome ''scoop'') -Force | Out-Null'
        'New-Item -ItemType Directory -Path (Join-Path $rustConfigHome ''scoop'') -Force | Out-Null'
        'Set-Content -Path (Join-Path $upstreamConfigHome ''scoop\config.json'') -Value ''{}'' -Encoding Ascii'
        'Set-Content -Path (Join-Path $rustConfigHome ''scoop\config.json'') -Value ''{}'' -Encoding Ascii'
    )
    Set-Content -Path $Path -Value ($body -join "`r`n") -NoNewline -Encoding Ascii
}

cargo build --release --bin scoop | Out-Null
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
New-Item -ItemType Directory -Force -Path $wrapperDir | Out-Null
$state = Initialize-Fixture

$jsonPath = Join-Path $resultsDir "$Name.json"
$prepareScript = Join-Path $resultsDir "$Name-prepare.ps1"
$upstreamWrapper = Join-Path $wrapperDir "$Name-upstream.cmd"
$rustWrapper = Join-Path $wrapperDir "$Name-rust.cmd"

New-PrepareScript -Path $prepareScript -State $state

New-BenchmarkWrapper -Path $upstreamWrapper -LocalRoot $upstreamRoot -ConfigHome $upstreamConfigHome -CommandParts @(
    'pwsh',
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    (Join-Path $upstreamRoot 'apps\scoop\current\bin\scoop.ps1'),
    'update'
)
New-BenchmarkWrapper -Path $rustWrapper -LocalRoot $rustRoot -ConfigHome $rustConfigHome -CommandParts @(
    $rustExe,
    'update'
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
