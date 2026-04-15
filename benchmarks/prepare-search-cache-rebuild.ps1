$ErrorActionPreference = 'Stop'
$dbPath = "D:\Applications\Scoop\scoop.db"
$dbDir = Split-Path -Parent $dbPath
New-Item -ItemType Directory -Path $dbDir -Force | Out-Null
[System.IO.File]::WriteAllText($dbPath, 'not a sqlite database')