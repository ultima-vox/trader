param(
    [ValidateRange(10, 1440)]
    [int]$Minutes = 60,
    [ValidateRange(1, 60)]
    [int]$SampleSeconds = 5,
    [string]$Command = "cargo test --locked -p vox-runtime --test runtime_live runtime_idle_soak_in_sandbox -- --ignored --nocapture"
)

$ErrorActionPreference = "Stop"
$logicalCores = [Environment]::ProcessorCount
$os = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$baseline = Get-Process | Measure-Object -Property WorkingSet64 -Sum
Write-Output ("QUALIFICATION_ENV os={0} version={1} cpu={2} logical_cores={3} baseline_rss_mib={4:N1}" -f $os.Caption, $os.Version, $cpu.Name, $logicalCores, ($baseline.Sum / 1MB))

$runnerCommand = "if ([string]::IsNullOrWhiteSpace(`$env:TINVEST_SANDBOX_TOKEN)) { `$env:TINVEST_SANDBOX_TOKEN = [Environment]::GetEnvironmentVariable('TINVEST_SANDBOX_TOKEN', 'User') }; `$env:TINVEST_RUNTIME_SOAK_MINUTES='$Minutes'; $Command"
$process = Start-Process -FilePath "powershell.exe" -ArgumentList @("-NoProfile", "-Command", $runnerCommand) -PassThru -WindowStyle Hidden
$samples = [System.Collections.Generic.List[object]]::new()
$deadline = [DateTimeOffset]::UtcNow.AddMinutes($Minutes + 5)
$previousCpu = @{}
$previousAt = [DateTimeOffset]::UtcNow

while ([DateTimeOffset]::UtcNow -lt $deadline -and -not $process.HasExited) {
    Start-Sleep -Seconds $SampleSeconds
    $process.Refresh()
    if ($process.HasExited) { break }
    $now = [DateTimeOffset]::UtcNow
    $elapsed = ($now - $previousAt).TotalSeconds
    $targets = Get-Process | Where-Object { $_.ProcessName -like "runtime_live-*" }
    if (-not $targets) { continue }
    $cpuDelta = 0.0
    $rss = 0L
    foreach ($target in $targets) {
        $currentCpu = $target.TotalProcessorTime.TotalSeconds
        $priorCpu = if ($previousCpu.ContainsKey($target.Id)) { $previousCpu[$target.Id] } else { $currentCpu }
        $cpuDelta += [Math]::Max(0.0, $currentCpu - $priorCpu)
        $previousCpu[$target.Id] = $currentCpu
        $rss += $target.WorkingSet64
    }
    $oneCorePercent = if ($elapsed -gt 0) { 100.0 * $cpuDelta / $elapsed } else { 0.0 }
    $samples.Add([pscustomobject]@{ At = $now; Cpu = $oneCorePercent; RssMiB = $rss / 1MB })
    $previousAt = $now
}

if (-not $process.HasExited) {
    Stop-Process -Id $process.Id
    throw "runtime soak exceeded requested observation without clean exit"
}
if ($process.ExitCode -ne 0) { throw "runtime qualification exited $($process.ExitCode)" }
if ($samples.Count -eq 0) { throw "runtime qualification produced no resource samples" }

$warm = $samples | Select-Object -Skip ([Math]::Min(12, $samples.Count - 1))
$averageCpu = ($warm | Measure-Object -Property Cpu -Average).Average
$maxRss = ($warm | Measure-Object -Property RssMiB -Maximum).Maximum
$rssGrowth = $warm[-1].RssMiB - $warm[0].RssMiB
Write-Output ("RESOURCE_SUMMARY samples={0} avg_one_core_cpu_percent={1:N2} max_rss_mib={2:N1} post_warmup_rss_growth_mib={3:N1}" -f $samples.Count, $averageCpu, $maxRss, $rssGrowth)

if ($averageCpu -gt 2.0) { throw "idle CPU budget exceeded" }
if ($maxRss -gt 150.0) { throw "RSS target exceeded" }
if ($rssGrowth -gt 20.0) { throw "RSS growth budget exceeded" }
