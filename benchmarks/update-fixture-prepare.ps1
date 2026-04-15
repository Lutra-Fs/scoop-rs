$ErrorActionPreference = 'Stop'
$upstreamRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-update-bench-9d7ec961-0b58-4f23-9a86-af7f9c61538b\upstream-local"
$rustRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-update-bench-9d7ec961-0b58-4f23-9a86-af7f9c61538b\rust-local"
$globalRoot = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-update-bench-9d7ec961-0b58-4f23-9a86-af7f9c61538b\global"
$upstreamConfigHome = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-update-bench-9d7ec961-0b58-4f23-9a86-af7f9c61538b\upstream-config"
$rustConfigHome = "C:\Users\lutra\AppData\Local\Temp\scoop-rs-update-bench-9d7ec961-0b58-4f23-9a86-af7f9c61538b\rust-config"
$upstreamCoreHead = "15fd2dd03e1f494513767f7fc7bb74d59ed11c7e"
$upstreamBucketHead = "198bcb77bb30d303c66c66aefd4ee350924bfe1c"
$rustBucketHead = "dc55935beee834a264535c47973c5fe32971d546"
& git -C (Join-Path $upstreamRoot 'apps\scoop\current') reset --hard $upstreamCoreHead -q | Out-Null
& git -C (Join-Path $upstreamRoot 'apps\scoop\current') clean -fdqx | Out-Null
& git -C (Join-Path $upstreamRoot 'buckets\main') reset --hard $upstreamBucketHead -q | Out-Null
& git -C (Join-Path $upstreamRoot 'buckets\main') clean -fdqx | Out-Null
& git -C (Join-Path $rustRoot 'buckets\main') reset --hard $rustBucketHead -q | Out-Null
& git -C (Join-Path $rustRoot 'buckets\main') clean -fdqx | Out-Null
$rustApps = Join-Path $rustRoot 'apps'
if (Test-Path -LiteralPath $rustApps) { cmd /c rd /s /q "$rustApps" | Out-Null }
New-Item -ItemType Directory -Path (Join-Path $rustRoot 'apps\scoop\1.0.0') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $rustRoot 'apps\scoop\current') -Force | Out-Null
Set-Content -Path (Join-Path $rustRoot 'apps\scoop\1.0.0\install.json') -Value '{"bucket":"main","architecture":"64bit"}' -Encoding Ascii
Set-Content -Path (Join-Path $rustRoot 'apps\scoop\1.0.0\manifest.json') -Value '{"version":"1.0.0"}' -Encoding Ascii
Set-Content -Path (Join-Path $rustRoot 'apps\scoop\1.0.0\scoop.exe') -Value 'old binary' -Encoding Ascii
Set-Content -Path (Join-Path $rustRoot 'apps\scoop\current\manifest.json') -Value '{"version":"1.0.0"}' -Encoding Ascii
foreach ($root in @($upstreamRoot, $rustRoot, $globalRoot)) {
    $shims = Join-Path $root 'shims'
    if (Test-Path -LiteralPath $shims) { cmd /c rd /s /q "$shims" | Out-Null }
    New-Item -ItemType Directory -Path $shims -Force | Out-Null
}
New-Item -ItemType Directory -Path (Join-Path $upstreamConfigHome 'scoop') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $rustConfigHome 'scoop') -Force | Out-Null
Set-Content -Path (Join-Path $upstreamConfigHome 'scoop\config.json') -Value '{}' -Encoding Ascii
Set-Content -Path (Join-Path $rustConfigHome 'scoop\config.json') -Value '{}' -Encoding Ascii