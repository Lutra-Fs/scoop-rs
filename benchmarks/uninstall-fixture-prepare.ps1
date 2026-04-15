$ErrorActionPreference = 'Stop'
$localRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-uninstall-bench-bbaf4775-156f-4aef-a68b-7dfee61c5f9a\local"
$globalRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-uninstall-bench-bbaf4775-156f-4aef-a68b-7dfee61c5f9a\global"
$configHome = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-uninstall-bench-bbaf4775-156f-4aef-a68b-7dfee61c5f9a\config"
$upstreamInstall = "D:\Applications\Scoop\apps\scoop\current\bin\scoop.ps1"
$resetTargets = @(
    (Join-Path $localRoot 'apps'),
    (Join-Path $localRoot 'shims'),
    (Join-Path $localRoot 'cache'),
    (Join-Path $globalRoot 'apps'),
    (Join-Path $globalRoot 'shims'),
    (Join-Path $globalRoot 'cache')
)
foreach ($target in $resetTargets) {
    if (Test-Path -LiteralPath $target) {
        cmd /c rd /s /q "$target" | Out-Null
    }
}
New-Item -ItemType Directory -Path (Join-Path $localRoot 'apps') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $localRoot 'shims') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $globalRoot 'apps') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $globalRoot 'shims') -Force | Out-Null
$env:SCOOP = $localRoot
$env:SCOOP_GLOBAL = $globalRoot
$env:XDG_CONFIG_HOME = $configHome
& pwsh -NoProfile -ExecutionPolicy Bypass -File $upstreamInstall install demo --no-update-scoop *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'fixture install failed'
}