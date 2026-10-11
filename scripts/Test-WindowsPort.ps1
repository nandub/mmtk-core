#Requires -Version 5.1
<#
.SYNOPSIS
    Validates the MMTk Windows backend using the malloc_mimalloc feature.
.DESCRIPTION
    Runs Rust formatting, library compilation, strict Clippy, and serial/parallel
    tests. Stops on the first nonzero native exit code. Writes logs to a chosen
    directory outside the source tree by default.
.EXAMPLE
    .\scripts\Test-WindowsPort.ps1 -Verbose
.EXAMPLE
    .\scripts\Test-WindowsPort.ps1 -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter()]
    [string]$RepositoryPath = (Split-Path -Parent $PSScriptRoot),

    [Parameter()]
    [string]$LogDirectory = (Join-Path $env:TEMP 'mmtk-windows-validation'),

    [Parameter()]
    [ValidateRange(1, 50)]
    [int]$RepeatParallel = 3,

    [Parameter()]
    [ValidateRange(1, 3600)]
    [int]$StepTimeoutSeconds = 600
)

begin {
    $ErrorActionPreference = 'Stop'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found on PATH.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $RepositoryPath 'Cargo.toml') -PathType Leaf)) {
        throw "Cargo.toml not found at $RepositoryPath"
    }
    $steps = @(
        @{ Name = 'fmt'; Args = @('fmt', '--all', '--', '--check') },
        @{ Name = 'check'; Args = @('check', '--lib', '--features', 'malloc_mimalloc') },
        @{ Name = 'clippy'; Args = @('clippy', '--lib', '--features', 'malloc_mimalloc', '--', '-D', 'warnings') },
        @{ Name = 'serial'; Args = @('test', '--features', 'malloc_mimalloc', '--', '--test-threads=1') }
    )
    for ($i = 1; $i -le $RepeatParallel; $i++) {
        $steps += @{ Name = ('parallel-{0:D2}' -f $i); Args = @('test', '--features', 'malloc_mimalloc', '--', '--test-threads=4') }
    }
}
process {
    $originalLocation = Get-Location
    try {
        foreach ($step in $steps) {
            $log = Join-Path $LogDirectory ($step.Name + '.log')
            $description = 'cargo ' + ($step.Args -join ' ')
            if (-not $PSCmdlet.ShouldProcess($RepositoryPath, $description)) {
                continue
            }
            if (-not (Test-Path -LiteralPath $LogDirectory -PathType Container)) {
                New-Item -Path $LogDirectory -ItemType Directory -Force | Out-Null
            }
            Set-Location -LiteralPath $RepositoryPath
            Write-Verbose $description
            $commandArgs = [string[]]$step.Args
            # PowerShell 5.1 promotes native stderr output to NativeCommandError
            # when ErrorActionPreference is Stop. Cargo writes normal progress
            # messages to stderr, so avoid the PowerShell native pipeline.
            $stdoutLog = Join-Path $LogDirectory ($step.Name + '.stdout.log')
            $stderrLog = Join-Path $LogDirectory ($step.Name + '.stderr.log')
            $cargoExe = (Get-Command cargo.exe -CommandType Application -ErrorAction Stop).Source
            Write-Host ('[{0}] START {1}' -f (Get-Date -Format 'HH:mm:ss'), $description)
            $watch = [Diagnostics.Stopwatch]::StartNew()
            # Use the .NET Process API. Windows PowerShell 5.1 may return a
            # Start-Process -PassThru object whose ExitCode is $null even
            # after WaitForExit(); a blank ExitCode must never indicate failure.
            $startInfo = New-Object System.Diagnostics.ProcessStartInfo
            $startInfo.FileName = $cargoExe
            $startInfo.Arguments = ($commandArgs -join ' ')
            $startInfo.WorkingDirectory = $RepositoryPath
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.RedirectStandardOutput = $true
            $startInfo.RedirectStandardError = $true
            $child = New-Object System.Diagnostics.Process
            $child.StartInfo = $startInfo
            $stdoutStream = $null
            $stderrStream = $null
            $outTask = $null
            $errTask = $null
            $exit = $null
            try {
                $stdoutStream = [System.IO.File]::Create($stdoutLog)
                $stderrStream = [System.IO.File]::Create($stderrLog)
                if (-not $child.Start()) {
                    throw "Unable to start cargo.exe: $description"
                }
                # Drain both pipes concurrently; waiting before reading can
                # deadlock when Cargo or the test harness emits enough output.
                $outTask = $child.StandardOutput.BaseStream.CopyToAsync($stdoutStream)
                $errTask = $child.StandardError.BaseStream.CopyToAsync($stderrStream)
                if (-not $child.WaitForExit($StepTimeoutSeconds * 1000)) {
                    try { $child.Kill() } catch { Write-Verbose $_.Exception.Message }
                    try { $child.WaitForExit(5000) | Out-Null } catch {}
                    throw "Timed out after $StepTimeoutSeconds seconds: $description; stdout=$stdoutLog; stderr=$stderrLog"
                }
                # Copy tasks must complete before closing the destination files.
                [System.Threading.Tasks.Task]::WaitAll(
                    [System.Threading.Tasks.Task[]]@($outTask, $errTask)
                )
                $child.Refresh()
                $exit = [int]$child.ExitCode
            }
            finally {
                $watch.Stop()
                if ($null -ne $stdoutStream) { $stdoutStream.Dispose() }
                if ($null -ne $stderrStream) { $stderrStream.Dispose() }
                $child.Dispose()
            }
            Write-Host ('[{0}] END   {1} exit={2} duration={3:n1}s' -f `
                (Get-Date -Format 'HH:mm:ss'), $step.Name, $exit, $watch.Elapsed.TotalSeconds)
            # Preserve both streams without depending on stderr parsing.
            # These are grouped by stream, not guaranteed chronological.
            @('=== STDOUT ===') | Set-Content -LiteralPath $log -Encoding UTF8
            if (Test-Path -LiteralPath $stdoutLog) {
                Get-Content -LiteralPath $stdoutLog | Add-Content -LiteralPath $log -Encoding UTF8
            }
            '=== STDERR ===' | Add-Content -LiteralPath $log -Encoding UTF8
            if (Test-Path -LiteralPath $stderrLog) {
                Get-Content -LiteralPath $stderrLog | Add-Content -LiteralPath $log -Encoding UTF8
            }
            $result = [pscustomobject]@{
                Step = $step.Name
                ExitCode = $exit
                Passed = ($exit -eq 0)
                Log = $log
            }
            Write-Output $result
            if ($exit -ne 0) {
                Get-Content -LiteralPath $log -Tail 25 | Out-Host
                throw "$description failed with exit code $exit. See $log"
            }
        }
    }
    finally {
        Set-Location $originalLocation
    }
}
end {}
