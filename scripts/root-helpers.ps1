$ErrorActionPreference = 'Stop'

function Resolve-ScoopRoot {
    param(
        [string]$ScoopRoot
    )

    if (-not [string]::IsNullOrWhiteSpace($ScoopRoot)) {
        return $ScoopRoot
    }
    if (-not [string]::IsNullOrWhiteSpace($env:SCOOP)) {
        return $env:SCOOP
    }
    if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
        return (Join-Path $env:USERPROFILE 'scoop')
    }

    throw 'Unable to resolve Scoop root. Pass -ScoopRoot or set SCOOP and USERPROFILE.'
}

function Resolve-UpstreamAppRoot {
    param(
        [string]$ScoopRoot
    )

    return (Join-Path (Resolve-ScoopRoot -ScoopRoot $ScoopRoot) 'apps\scoop\current')
}
