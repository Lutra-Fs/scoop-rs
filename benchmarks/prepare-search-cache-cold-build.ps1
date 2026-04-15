$ErrorActionPreference = 'Stop'
$dbPath = "D:\Applications\Scoop\scoop.db"
if (Test-Path -LiteralPath $dbPath) { Remove-Item -LiteralPath $dbPath -Force }