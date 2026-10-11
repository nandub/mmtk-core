#requires -Version 5.1
<#
.SYNOPSIS
    Generates an MMTk Windows/CoreCLR integration readiness inventory.
.DESCRIPTION
    Inspects an MMTk checkout and local development environment and writes a
    JSON evidence inventory. Does not certify CoreCLR compatibility.
    With -RunQualification, also executes the existing Windows qualification
    script, which may create or modify Cargo build artifacts.
.PARAMETER RepositoryPath
    Path to the MMTk Core checkout.
.PARAMETER OutputDirectory
    Directory for readiness JSON reports. Defaults to LocalAppData\MMTk\Readiness.
.PARAMETER RunQualification
    Run the existing Windows qualification suite after inventory generation.
.EXAMPLE
    .\Test-MMTkWindowsReadiness.ps1 -RepositoryPath 'S:\Sources\mmtk-core-windows' -Verbose
.EXAMPLE
    .\Test-MMTkWindowsReadiness.ps1 -RepositoryPath 'S:\Sources\mmtk-core-windows' -OutputDirectory "$env:OneDrive\MMTk\Readiness" -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$RepositoryPath,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$OutputDirectory = (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'MMTk\Readiness'),

    [Parameter()]
    [switch]$RunQualification
)

begin {
    Set-StrictMode -Version Latest
    $ErrorActionPreference = 'Stop'

    function Invoke-NativeCapture {
        [CmdletBinding()]
        param(
            [Parameter(Mandatory = $true)]
            [ValidateNotNullOrEmpty()]
            [string]$Executable,

            [Parameter()]
            [string[]]$Arguments = @(),

            [Parameter(Mandatory = $true)]
            [ValidateNotNullOrEmpty()]
            [string]$WorkingDirectory,

            [Parameter()]
            [ValidateRange(1, 3600)]
            [int]$TimeoutSeconds = 120
        )

        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $Executable
        $psi.WorkingDirectory = $WorkingDirectory
        $psi.UseShellExecute = $false
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.CreateNoWindow = $true

        # ProcessStartInfo.ArgumentList is unavailable in Windows PowerShell 5.1's
        # .NET Framework. These inventory probes use only simple, fixed arguments.
        foreach ($argument in $Arguments) {
            if ($null -eq $argument -or $argument -match '[\s"]') {
                throw "Unsupported native argument in readiness probe: '$argument'"
            }
        }
        $psi.Arguments = ($Arguments -join ' ')

        $process = New-Object System.Diagnostics.Process
        $process.StartInfo = $psi
        try {
            if (-not $process.Start()) {
                throw "Failed to start native command: $Executable"
            }

            # Read both pipes concurrently to avoid stdout/stderr deadlocks.
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()

            if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
                try {
                    $process.Kill()
                    [void]$process.WaitForExit(5000)
                }
                catch {
                    Write-Verbose "Unable to stop timed-out process: $_"
                }
                throw "Timed out running $Executable $($psi.Arguments)"
            }

            [System.Threading.Tasks.Task]::WaitAll(
                [System.Threading.Tasks.Task[]]@($stdoutTask, $stderrTask)
            )

            $result = [pscustomobject]@{
                ExitCode = [int]$process.ExitCode
                StdOut   = ([string]$stdoutTask.Result).Trim()
                StdErr   = ([string]$stderrTask.Result).Trim()
            }
            Write-Verbose ('Native: {0} {1}; exit={2}; stdout={3}; stderr={4}' -f `
                $Executable, $psi.Arguments, $result.ExitCode, $result.StdOut, $result.StdErr)
            return $result
        }
        finally {
            $process.Dispose()
        }
    }

    function Get-CommandVersion {
        [CmdletBinding()]
        param(
            [Parameter(Mandatory = $true)]
            [string]$Name,

            [Parameter()]
            [string[]]$Arguments = @('--version'),

            [Parameter(Mandatory = $true)]
            [string]$WorkingDirectory
        )

        $command = Get-Command -Name $Name -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -eq $command) {
            return [pscustomobject]@{ Found = $false; Version = $null; Error = 'Not in PATH' }
        }

        try {
            $result = Invoke-NativeCapture -Executable $command.Source `
                -Arguments $Arguments -WorkingDirectory $WorkingDirectory
            return [pscustomobject]@{
                Found   = ($result.ExitCode -eq 0)
                Version = if ($result.ExitCode -eq 0) { $result.StdOut } else { $null }
                Error   = if ($result.ExitCode -eq 0) { $null } else { $result.StdErr }
            }
        }
        catch {
            return [pscustomobject]@{
                Found   = $false
                Version = $null
                Error   = $_.Exception.Message
            }
        }
    }
}

process {
    $resolved = (Resolve-Path -LiteralPath $RepositoryPath -ErrorAction Stop).ProviderPath
    if (-not (Test-Path -LiteralPath (Join-Path $resolved 'Cargo.toml') -PathType Leaf)) {
        throw "Not an MMTk Cargo repository: $resolved"
    }

    $paths = [ordered]@{
        WindowsBackend       = 'src\util\os\imp\windows.rs'
        RawMemoryFreeList    = 'src\util\raw_memory_freelist.rs'
        WindowsStressTests  = 'src\util\os\imp\windows_stress_tests.rs'
        WindowsQualification = 'scripts\Test-WindowsPort.ps1'
        LinuxQualification  = 'scripts\test-linux-port.sh'
        AgentGuidance       = 'AGENTS.md'
    }

    $files = @(
        foreach ($entry in $paths.GetEnumerator()) {
            $path = Join-Path $resolved $entry.Value
            $exists = Test-Path -LiteralPath $path -PathType Leaf
            [pscustomobject]@{
                Name         = $entry.Key
                RelativePath = $entry.Value
                Exists       = [bool]$exists
                SHA256       = if ($exists) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash } else { $null }
            }
        }
    )

    $git = Get-CommandVersion -Name 'git.exe' -WorkingDirectory $resolved
    $cargo = Get-CommandVersion -Name 'cargo.exe' -WorkingDirectory $resolved
    $rustc = Get-CommandVersion -Name 'rustc.exe' -WorkingDirectory $resolved
    $dotnet = Get-CommandVersion -Name 'dotnet.exe' -WorkingDirectory $resolved

    $commit = $null
    $branch = $null
    $dirty = $null
    if ($git.Found) {
        $gitExecutable = (Get-Command -Name 'git.exe' -CommandType Application -ErrorAction Stop |
            Select-Object -First 1).Source

        # WorkingDirectory is explicitly set, avoiding fragile -C quoting on
        # .NET Framework when repository paths contain spaces.
        $c = Invoke-NativeCapture -Executable $gitExecutable `
            -Arguments @('rev-parse', 'HEAD') -WorkingDirectory $resolved
        $b = Invoke-NativeCapture -Executable $gitExecutable `
            -Arguments @('branch', '--show-current') -WorkingDirectory $resolved
        $s = Invoke-NativeCapture -Executable $gitExecutable `
            -Arguments @('status', '--porcelain') -WorkingDirectory $resolved

        foreach ($result in @($c, $b, $s)) {
            if ($result.ExitCode -ne 0) {
                throw "Git metadata probe failed (exit $($result.ExitCode)): $($result.StdErr)"
            }
        }
        if ([string]::IsNullOrWhiteSpace($c.StdOut)) {
            throw 'Git rev-parse returned an empty commit hash.'
        }
        $commit = $c.StdOut
        $branch = $b.StdOut
        $dirty = -not [string]::IsNullOrWhiteSpace($s.StdOut)
    }
    else {
        Write-Warning "Git metadata unavailable: $($git.Error)"
    }

    $backend = Join-Path $resolved $paths.WindowsBackend
    $markers = [ordered]@{}
    if (Test-Path -LiteralPath $backend -PathType Leaf) {
        $source = [System.IO.File]::ReadAllText($backend)
        foreach ($pattern in @('VirtualAlloc', 'VirtualFree', 'VirtualQuery', 'VirtualProtect',
            'MEM_RESERVE', 'MEM_COMMIT', 'MEM_DECOMMIT', 'MEM_RELEASE')) {
            $markers[$pattern] = $source.Contains($pattern)
        }
    }

    $report = [ordered]@{
        SchemaVersion       = 1
        GeneratedUtc        = [DateTime]::UtcNow.ToString('o')
        QualificationStatus = 'NOT ASSESSED - inspection only'
        Repository          = $resolved
        GitCommit           = $commit
        GitBranch           = $branch
        WorkingTreeDirty    = $dirty
        OperatingSystem     = [Environment]::OSVersion.VersionString
        ProcessArchitecture = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITECTURE')
        PowerShell          = $PSVersionTable.PSVersion.ToString()
        Tools               = [ordered]@{ Git = $git; Cargo = $cargo; Rustc = $rustc; DotNet = $dotnet }
        Files               = $files
        BackendMarkers      = $markers
        UnverifiedContracts = @(
            'Side-metadata virtual address reservation capacity and runtime base initialization'
            'Reservation ownership and concurrent map/unmap safety'
            'Fixed-address replacement and Windows allocation granularity'
            'Partial release, decommit/recommit, and failure atomicity'
            'MMTk VMBinding integration and CoreCLR GC interface ABI'
            'CoreCLR heap/object/reference layout and pinning compatibility'
            'CoreCLR stop-the-world handshake, roots, write barriers, finalization'
            'Real CoreCLR bootstrap and NoGC smoke test'
        )
    }

    $obj = [pscustomobject]$report
    if ($PSCmdlet.ShouldProcess($OutputDirectory, 'Create readiness JSON report')) {
        New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
        $file = Join-Path $OutputDirectory ('mmtk-windows-readiness-{0}.json' -f `
            ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmssfff')))
        $obj | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath $file -Encoding UTF8
        Write-Verbose "Readiness inventory saved: $file"
        $obj | Add-Member -NotePropertyName ReportPath -NotePropertyValue $file
    }

    if ($RunQualification) {
        $runner = Join-Path $resolved $paths.WindowsQualification
        if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
            throw 'Windows qualification runner not found.'
        }
        if ($PSCmdlet.ShouldProcess($resolved, 'Run existing Windows qualification script')) {
            & $runner -RepeatParallel 30 -StepTimeoutSeconds 600
            # The qualification runner is expected to throw on failure.
        }
    }

    Write-Output $obj
}
end {
}
