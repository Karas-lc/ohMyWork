# Measures indexing and search APIs only. Not a GUI performance acceptance run.
param(
    [Parameter(Mandatory=$true)][string]$CorpusDir,
    [switch]$Smoke,
    [switch]$Full,
    [ValidateRange(1,30)][int]$Runs = 3,
    [ValidateRange(1,100)][int]$Queries = 100
)
$ErrorActionPreference = "Stop"
if ($Smoke -and $Full) { throw "Choose -Smoke or -Full, not both" }
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    if (-not (Test-Path "$CorpusDir\MANIFEST.json")) {
        if ($Smoke) { python tools/generate-corpus.py --out $CorpusDir --smoke --seed 424242 }
        elseif ($Full) { python tools/generate-corpus.py --out $CorpusDir --seed 424242 }
        else { throw "Specify -Smoke or -Full when generating a corpus" }
        if ($LASTEXITCODE -ne 0) { throw "Corpus generation failed: $LASTEXITCODE" }
    }
    $manifest = Get-Content "$CorpusDir\MANIFEST.json" -Raw | ConvertFrom-Json
    Write-Host ("Manifest: notes={0} tasks={1} ~{2}MB" -f $manifest.note_count, $manifest.task_count, $manifest.total_mb)
    $messages = @(cargo build -p ohmywork --bin perf_runner --release --locked --message-format=json)
    if ($LASTEXITCODE -ne 0) { throw "Performance runner build failed: $LASTEXITCODE" }
    $executable = $null
    foreach ($line in $messages) {
        $artifact = $line | ConvertFrom-Json
        if ($artifact.reason -eq "compiler-artifact" -and $artifact.target.name -eq "perf_runner" -and $artifact.executable) { $executable = $artifact.executable }
    }
    if (-not $executable) { throw "Cargo did not report a perf_runner executable" }
    & $executable --corpus $CorpusDir --runs $Runs --queries $Queries
    if ($LASTEXITCODE -ne 0) { throw "Performance runner failed: $LASTEXITCODE" }
    Write-Host "API-only report: tools/perf-results-latest.txt. GUI budgets remain unmeasured."
} finally { Pop-Location }
