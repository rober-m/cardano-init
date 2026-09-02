param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string] $LockPath,

    [Parameter(Mandatory = $true, Position = 1)]
    [string] $EnvPath,

    [Parameter(Mandatory = $true, Position = 2)]
    [string] $Context,

    [Parameter(Position = 3, ValueFromRemainingArguments = $true)]
    [string[]] $Mappings
)

$ErrorActionPreference = "Stop"
if ($Mappings.Count -eq 0 -or $Mappings.Count % 2 -ne 0) {
    throw "Expected target/source environment mapping pairs"
}
for ($index = 0; $index -lt $Mappings.Count; $index += 2) {
    $target = $Mappings[$index]
    $source = $Mappings[$index + 1]
    if ($target -notmatch '^[A-Za-z_][A-Za-z0-9_]*$' -or
        $source -notmatch '^[A-Za-z_][A-Za-z0-9_]*$') {
        throw "Invalid environment mapping: $target/$source"
    }
}

$lockFullPath = [IO.Path]::GetFullPath($LockPath)
$envFullPath = [IO.Path]::GetFullPath($EnvPath)
if (-not [string]::Equals(
    $lockFullPath,
    "$envFullPath.lock",
    [StringComparison]::OrdinalIgnoreCase
)) {
    throw "Lock path must be the environment path plus .lock"
}
$deadline = [DateTime]::UtcNow.AddSeconds(30)
$lock = $null

while ($null -eq $lock) {
    try {
        $lock = [IO.File]::Open(
            $lockFullPath,
            [IO.FileMode]::OpenOrCreate,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::None
        )
    }
    catch [IO.IOException] {
        if ([DateTime]::UtcNow -ge $deadline) {
            [Console]::Error.WriteLine("Timed out waiting for {0}", $LockPath)
            exit 1
        }
        Start-Sleep -Milliseconds 100
    }
}

$tempPath = $null
$backupPath = $null
try {
    $lockDirectory = [IO.Path]::GetDirectoryName($lockFullPath)
    $lockName = [IO.Path]::GetFileName($lockFullPath)
    foreach ($stalePath in [IO.Directory]::GetFiles($lockDirectory, "$lockName.tmp.*")) {
        [IO.File]::Delete($stalePath)
    }

    $collectorScript = @'
set -e
context_env="$(cardano-up context env --context "$1")"
eval "$context_env"
shift
while [ "$#" -gt 0 ]; do
    target="$1"
    source="$2"
    shift 2
    eval "value=\${$source}"
    printf '%s=%s\n' "$target" "$value"
done
'@

    $consoleOutputEncoding = [Console]::OutputEncoding
    try {
        [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
        $mappedLines = @(& sh -c $collectorScript cardano-init-env $Context @Mappings)
    }
    finally {
        [Console]::OutputEncoding = $consoleOutputEncoding
    }
    $collectorExitCode = $LASTEXITCODE
    if ($collectorExitCode -ne 0) {
        exit $collectorExitCode
    }
    if ($mappedLines.Count -ne $Mappings.Count / 2) {
        throw "Environment value collector returned an unexpected line count"
    }

    $updates = [Collections.Specialized.OrderedDictionary]::new()
    for ($index = 0; $index -lt $Mappings.Count; $index += 2) {
        $target = $Mappings[$index]
        $prefix = "$target="
        $line = $mappedLines[$index / 2]
        if (-not $line.StartsWith($prefix, [StringComparison]::Ordinal)) {
            throw "Environment value collector returned an unexpected mapping"
        }
        if ($updates.Contains($target)) {
            throw "Duplicate environment target: $target"
        }
        $updates.Add($target, $line.Substring($prefix.Length))
    }

    $original = if ([IO.File]::Exists($envFullPath)) {
        [IO.File]::ReadAllText($envFullPath)
    }
    else {
        ""
    }
    $lines = if ($original.Length -gt 0) {
        [Text.RegularExpressions.Regex]::Split($original, "\r?\n")
    }
    else {
        @()
    }
    $lineCount = $lines.Count
    if ($lineCount -gt 0 -and $lines[$lineCount - 1] -eq "" -and $original.EndsWith("`n")) {
        $lineCount--
    }

    $outputLines = [Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt $lineCount; $index++) {
        $line = $lines[$index]
        $owned = $false
        foreach ($target in $updates.Keys) {
            if ($line.StartsWith("$target=", [StringComparison]::Ordinal)) {
                $owned = $true
                break
            }
        }
        if (-not $owned) {
            $outputLines.Add($line)
        }
    }
    foreach ($target in $updates.Keys) {
        $outputLines.Add("$target=$($updates[$target])")
    }

    $text = [string]::Join("`n", $outputLines)
    if ($text.Length -gt 0) {
        $text += "`n"
    }
    $tempPath = "$lockFullPath.tmp.$([IO.Path]::GetRandomFileName())"
    $tempStream = [IO.File]::Open(
        $tempPath,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    try {
        $bytes = ([Text.UTF8Encoding]::new($false)).GetBytes($text)
        $tempStream.Write($bytes, 0, $bytes.Length)
        $tempStream.Flush($true)
    }
    finally {
        $tempStream.Dispose()
    }

    if ([IO.File]::Exists($envFullPath)) {
        $backupPath = "$lockFullPath.tmp.backup.$([IO.Path]::GetRandomFileName())"
        [IO.File]::Replace($tempPath, $envFullPath, $backupPath)
        $tempPath = $null
        [IO.File]::Delete($backupPath)
        $backupPath = $null
    }
    else {
        [IO.File]::Move($tempPath, $envFullPath)
        $tempPath = $null
    }
}
finally {
    if ($null -ne $tempPath -and [IO.File]::Exists($tempPath)) {
        [IO.File]::Delete($tempPath)
    }
    if ($null -ne $backupPath -and [IO.File]::Exists($backupPath)) {
        [IO.File]::Delete($backupPath)
    }
    $lock.Dispose()
}
