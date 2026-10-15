# Refuse a Windows binary that needs the Visual C++ runtime.
#
# Windows itself carries the C runtime (the UCRT, api-ms-win-crt-*.dll and
# ucrtbase.dll) but not the Visual C++ runtime: vcruntime140.dll,
# msvcp140.dll and friends come with the Visual C++ Redistributable, which a
# clean Windows does not have. A binary that imports one of them does not
# start there; Windows says "MSVCP140.dll was not found" and that is all the
# user sees. Our binaries link that runtime in (.cargo/config.toml sets
# crt-static for the Windows targets), and this checks that they did.
#
# Reads the file's import and delay-load tables itself, so it needs no
# Visual Studio tools. A file that is not a Windows binary is left alone.
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string[]] $Files
)

$ErrorActionPreference = 'Stop'

$forbidden = '^((msvcp|vcruntime|concrt|vcomp|vccorlib|mfc)\d+.*|ucrtbased)\.dll$'

function Get-PeImports([string] $path) {
    $b = [IO.File]::ReadAllBytes($path)
    if ($b.Length -lt 64 -or $b[0] -ne 0x4D -or $b[1] -ne 0x5A) { return $null }
    $pe = [BitConverter]::ToInt32($b, 0x3C)
    if ($pe -lt 0 -or $pe + 24 -gt $b.Length -or [BitConverter]::ToUInt32($b, $pe) -ne 0x4550) { return $null }
    $coff = $pe + 4
    $sections = [BitConverter]::ToUInt16($b, $coff + 2)
    $optSize = [BitConverter]::ToUInt16($b, $coff + 16)
    $opt = $coff + 20
    $magic = [BitConverter]::ToUInt16($b, $opt)
    $dirs = if ($magic -eq 0x20B) { $opt + 112 } else { $opt + 96 }
    $table = $opt + $optSize

    $toOffset = {
        param([uint32] $rva)
        for ($i = 0; $i -lt $sections; $i++) {
            $s = $table + 40 * $i
            $vsize = [BitConverter]::ToUInt32($b, $s + 8)
            $va = [BitConverter]::ToUInt32($b, $s + 12)
            $raw = [BitConverter]::ToUInt32($b, $s + 16)
            $ptr = [BitConverter]::ToUInt32($b, $s + 20)
            if ($rva -ge $va -and $rva -lt $va + [Math]::Max($vsize, $raw)) { return [int]($rva - $va + $ptr) }
        }
        return -1
    }
    $readName = {
        param([uint32] $rva)
        $at = & $toOffset $rva
        if ($at -lt 0) { return $null }
        $end = $at
        while ($end -lt $b.Length -and $b[$end] -ne 0) { $end++ }
        return [Text.Encoding]::ASCII.GetString($b, $at, $end - $at)
    }

    $names = New-Object System.Collections.Generic.List[string]
    # Imports: 20-byte descriptors, the name's address at +12, ended by zeros.
    $at = & $toOffset ([BitConverter]::ToUInt32($b, $dirs + 8))
    if ([BitConverter]::ToUInt32($b, $dirs + 8) -ne 0 -and $at -ge 0) {
        while ($at + 20 -le $b.Length) {
            $name = [BitConverter]::ToUInt32($b, $at + 12)
            if ($name -eq 0) { break }
            $n = & $readName $name
            if ($n) { $names.Add($n) }
            $at += 20
        }
    }
    # Delay-loaded imports: 32-byte descriptors, the name's address at +4.
    $at = & $toOffset ([BitConverter]::ToUInt32($b, $dirs + 13 * 8))
    if ([BitConverter]::ToUInt32($b, $dirs + 13 * 8) -ne 0 -and $at -ge 0) {
        while ($at + 32 -le $b.Length) {
            $name = [BitConverter]::ToUInt32($b, $at + 4)
            if ($name -eq 0) { break }
            $n = & $readName $name
            if ($n) { $names.Add($n) }
            $at += 32
        }
    }
    return $names
}

$bad = @()
foreach ($file in $Files) {
    $imports = Get-PeImports $file
    if ($null -eq $imports) { continue }
    $needs = @($imports | Where-Object { $_ -match $forbidden } | Sort-Object -Unique)
    if ($needs.Count -gt 0) {
        $bad += "$file needs $($needs -join ', ')"
    } else {
        Write-Host "check-windows-imports: $(Split-Path $file -Leaf) needs nothing beyond Windows ($($imports.Count) libraries)"
    }
}
if ($bad.Count -gt 0) {
    throw "check-windows-imports: the Visual C++ runtime is not on a clean Windows, so these would not start there: $($bad -join '; ')"
}
