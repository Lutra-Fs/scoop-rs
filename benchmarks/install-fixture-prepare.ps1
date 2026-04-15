$ErrorActionPreference = 'Stop'
$localRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-install-bench-216d5a45-167e-4527-b481-e4ebd35abcc2\local"
$globalRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-install-bench-216d5a45-167e-4527-b481-e4ebd35abcc2\global"
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