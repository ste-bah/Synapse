$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$root = "C:\code\Synapse"
$promptPath = Join-Path $root "tmp\issue727\synapse_spawned_primary_agent_prompt.txt"
$runLog = Join-Path $root "tmp\issue727\synapse_spawned_primary_agent_run.jsonl"
$errLog = Join-Path $root "tmp\issue727\synapse_spawned_primary_agent_stderr.log"
$exitPath = Join-Path $root "tmp\issue727\synapse_spawned_primary_agent_exit.txt"

Set-Location $root
Remove-Item -LiteralPath $runLog, $errLog, $exitPath -Force -ErrorAction SilentlyContinue

Get-Content -LiteralPath $promptPath -Raw |
    claude -p --verbose --output-format stream-json --dangerously-skip-permissions --add-dir $root > $runLog 2> $errLog

$exitCode = if ($null -eq $LASTEXITCODE) { 0 } else { $LASTEXITCODE }
Set-Content -LiteralPath $exitPath -Value $exitCode -Encoding ASCII
exit $exitCode
