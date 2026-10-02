#Requires -Version 7.2
<#
.SYNOPSIS
    ohMyWork 环境诊断（只读）。检测本机是否满足 V1 的 Windows Rust/GPUI 构建条件。

.DESCRIPTION
    只读检测：固定工具链（rust-toolchain.toml）、rustup/rustc/cargo、MSVC 与
    Windows SDK（通过 vswhere / 注册表 / 标准目录）、CMake、Dev Shell 加载状态，
    并在已加载的开发环境中编译一个临时 MSVC 空程序后清理。

    不会修改 PATH、全局 Cargo 配置、Pi 或任何用户环境；不会安装任何组件。
    缺失项只输出修复命令，由用户决定并显式执行。

.PARAMETER Json
    只输出一个 JSON 对象（含每项状态与退出码），便于 CI 机器判断。

.PARAMETER SkipCompile
    跳过 MSVC 空程序编译冒烟测试（例如在无 MSVC 的静态检查任务中使用）。

.EXAMPLE
    pwsh -NoProfile -File tools/doctor.ps1

.EXAMPLE
    pwsh -NoProfile -File tools/doctor.ps1 -Json -SkipCompile

.OUTPUTS
    退出码：0 = 全部 OK；1 = 存在 WARN（可构建但需注意）；2 = 存在 ERROR（缺必需组件）。
#>
[CmdletBinding()]
param(
    [switch]$Json,
    [switch]$SkipCompile
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# 7.3+ 默认把原生 stderr 当错误；本脚本需要收集原生输出自行判断，故关闭该行为。
$PSNativeCommandUseErrorActionPreference = $false
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

$repoRoot = Split-Path -Parent $PSScriptRoot   # 脚本位于 <仓库根>/tools/
$toolchainFile = Join-Path $repoRoot 'rust-toolchain.toml'
$script:checks = [System.Collections.Generic.List[object]]::new()

function Add-Check {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][ValidateSet('OK', 'WARN', 'ERROR', 'INFO')][string]$Status,
        [string]$Detail = '',
        [string]$Hint = ''
    )
    $script:checks.Add([pscustomobject]@{
        name   = $Name
        status = $Status
        detail = $Detail
        hint   = $Hint
    })
    if (-not $Json) {
        $tag = switch ($Status) {
            'OK'    { '[ OK ]' }
            'WARN'  { '[WARN]' }
            'ERROR' { '[ERR ]' }
            'INFO'  { '[....]' }
        }
        Write-Host ("{0} {1} — {2}" -f $tag, $Name, $Detail)
        if ($Status -eq 'ERROR' -and $Hint) { Write-Host ("       修复: {0}" -f $Hint) }
        if ($Status -eq 'WARN'  -and $Hint) { Write-Host ("       提示: {0}" -f $Hint) }
    }
}

# ---------------------------------------------------------------- 工具定位

function Find-Rustup {
    $cmd = Get-Command rustup -ErrorAction SilentlyContinue
    if ($null -ne $cmd) { return $cmd.Source }
    $candidates = @()
    if ($env:USERPROFILE) { $candidates += (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe') }
    if ($env:RUSTUP_HOME)  { $candidates += (Join-Path $env:RUSTUP_HOME 'bin\rustup.exe') }
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c)) { return $c }
    }
    return $null
}

function Find-CMake {
    # 返回 @{ path; version; inPath }；找不到返回 $null（不抛错）
    $cmd = Get-Command cmake -ErrorAction SilentlyContinue
    if ($null -ne $cmd) { return [pscustomobject]@{ path = $cmd.Source; inPath = $true } }
    $candidates = @(
        (Join-Path ${env:ProgramFiles} 'CMake\bin\cmake.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'CMake\bin\cmake.exe'),
        'C:\ProgramData\chocolatey\bin\cmake.exe'
    )
    if ($env:USERPROFILE) { $candidates += (Join-Path $env:USERPROFILE 'scoop\apps\cmake\current\bin\cmake.exe') }
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c)) { return [pscustomobject]@{ path = $c; inPath = $false } }
    }
    # Visual Studio 自带的 CMake 组件
    $vsCMake = @(
        (Join-Path ${env:ProgramFiles} 'Microsoft Visual Studio\*\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\*\*\CMake\bin\cmake.exe')
    )
    foreach ($pat in $vsCMake) {
        $hit = Get-ChildItem -Path (Split-Path -Parent $pat) -Filter 'cmake.exe' -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($hit) { return [pscustomobject]@{ path = $hit.FullName; inPath = $false } }
    }
    # WinGet 包安装位置
    if ($env:LOCALAPPDATA) {
        $wgRoot = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'
        if (Test-Path -LiteralPath $wgRoot) {
            $hit = Get-ChildItem -Path $wgRoot -Filter 'cmake.exe' -Recurse -Depth 8 -ErrorAction SilentlyContinue |
                Select-Object -First 1
            if ($hit) { return [pscustomobject]@{ path = $hit.FullName; inPath = $false } }
        }
    }
    return $null
}

function Find-VisualStudio {
    # 返回实例对象数组。每项: installPath, version, cl, link, rc, vcvars64, displayName, source
    $instances = [System.Collections.Generic.List[object]]::new()
    $vswhereCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'),
        (Join-Path ${env:ProgramFiles} 'Microsoft Visual Studio\Installer\vswhere.exe')
    )
    $vswhere = $vswhereCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if ($vswhere) {
        try {
            $raw = (& $vswhere -all -products '*' -format json) -join "`n"
            $parsed = $raw | ConvertFrom-Json
            foreach ($inst in @($parsed)) {
                [string]$ip = $inst.installationPath
                if (-not $ip) { continue }
                $msvcRoot = Join-Path $ip 'VC\Tools\MSVC'
                if (-not (Test-Path -LiteralPath $msvcRoot)) { continue }
                $verDir = Get-ChildItem -LiteralPath $msvcRoot -Directory -ErrorAction SilentlyContinue |
                    Sort-Object { [version]($_.Name -replace '^\D+', '') } -Descending |
                    Select-Object -First 1
                if (-not $verDir) { continue }
                $hostBin = Join-Path $verDir.FullName 'bin\Hostx64\x64'
                $vcvars = Join-Path $ip 'VC\Auxiliary\Build\vcvars64.bat'
                $instances.Add([pscustomobject]@{
                    installPath  = $ip
                    version      = $verDir.Name
                    cl           = Join-Path $hostBin 'cl.exe'
                    link         = Join-Path $hostBin 'link.exe'
                    rc           = Join-Path $hostBin 'rc.exe'
                    vcvars64     = $vcvars
                    displayName  = $inst.displayName
                    source       = 'vswhere'
                })
            }
        }
        catch {
            Add-Check 'Visual Studio 枚举' 'WARN' ("vswhere 解析失败: {0}" -f $_.Exception.Message) '继续用标准目录回退探测'
        }
    }
    else {
        Add-Check 'Visual Studio 枚举' 'INFO' '未找到 vswhere，改用标准目录探测'
    }
    if ($instances.Count -eq 0) {
        # 回退：标准目录 / 注册表
        $roots = @(
            (Join-Path ${env:ProgramFiles} 'Microsoft Visual Studio'),
            (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio')
        )
        foreach ($root in $roots | Where-Object { $_ -and (Test-Path -LiteralPath $_) }) {
            foreach ($edition in Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue) {
                $msvcRoot = Join-Path $edition.FullName 'VC\Tools\MSVC'
                if (-not (Test-Path -LiteralPath $msvcRoot)) { continue }
                $verDir = Get-ChildItem -LiteralPath $msvcRoot -Directory -ErrorAction SilentlyContinue |
                    Sort-Object { [version]($_.Name -replace '^\D+', '') } -Descending |
                    Select-Object -First 1
                if (-not $verDir) { continue }
                $hostBin = Join-Path $verDir.FullName 'bin\Hostx64\x64'
                $instances.Add([pscustomobject]@{
                    installPath = $edition.FullName
                    version     = $verDir.Name
                    cl          = Join-Path $hostBin 'cl.exe'
                    link        = Join-Path $hostBin 'link.exe'
                    rc          = Join-Path $hostBin 'rc.exe'
                    vcvars64    = Join-Path $edition.FullName 'VC\Auxiliary\Build\vcvars64.bat'
                    displayName = ''
                    source      = '标准目录'
                })
            }
        }
    }
    return @($instances)
}

function Get-WindowsSdk {
    # 返回 @{ version; root; rc } 或 $null
    $kitsRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
    if (-not (Test-Path -LiteralPath $kitsRoot)) { return $null }
    $incRoot = Join-Path $kitsRoot 'Include'
    if (-not (Test-Path -LiteralPath $incRoot)) { return $null }
    $verDir = Get-ChildItem -LiteralPath $incRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        Select-Object -First 1
    if (-not $verDir) { return $null }
    return [pscustomobject]@{
        version = $verDir.Name
        root    = $kitsRoot
        rc      = Join-Path $kitsRoot ("bin\{0}\x64\rc.exe" -f $verDir.Name)
        lib     = Join-Path $kitsRoot ("Lib\{0}\um\x64\kernel32.lib" -f $verDir.Name)
    }
}

# ---------------------------------------------------------------- 1. 工具链固定

if (Test-Path -LiteralPath $toolchainFile) {
    $toolchainContent = Get-Content -Raw -LiteralPath $toolchainFile
    if ($toolchainContent -match 'channel\s*=\s*"([^"]+)"') { $pin = $Matches[1] }
    if ($pin) {
        Add-Check '工具链固定' 'OK' ("rust-toolchain.toml 固定 channel {0}" -f $pin)
    }
    else {
        Add-Check '工具链固定' 'ERROR' 'rust-toolchain.toml 存在但未找到 channel 声明' '检查 TOML 语法'
    }
}
else {
    Add-Check '工具链固定' 'ERROR' '仓库根缺少 rust-toolchain.toml' '先创建 rust-toolchain.toml 固定工具链'
}

# ---------------------------------------------------------------- 2. rustup / 工具链

$rustupExe = Find-Rustup
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }

if ($null -eq $rustupExe) {
    Add-Check 'rustup' 'ERROR' 'PATH 及标准位置均未找到 rustup' '安装 Rust: 用 rustup-init 或 安装后重开终端（cargo 会随 rustup 提供）'
    Add-Check '工具链状态' 'WARN' 'rustup 缺失，跳过工具链检查'
}
else {
    # 注意：rustup >= 1.29 会在多数子命令（toolchain list 等）中对 rust-toolchain.toml 指定的
    # 缺失工具链自动安装。doctor 必须只读，因此绝不调用 rustup，只读取 rustup 管理的文件。
    Add-Check 'rustup' 'OK' ("发现 rustup: {0}（只读检测，不调用）" -f $rustupExe)

    $toolchainNames = @()
    if (Test-Path -LiteralPath (Join-Path $rustupHome 'toolchains')) {
        $toolchainNames = @(Get-ChildItem -LiteralPath (Join-Path $rustupHome 'toolchains') -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^[0-9A-Za-z_.\-]+$' } |
            ForEach-Object { $_.Name })
    }

    if ($pin) {
        if ($toolchainNames.Count -eq 0) {
            Add-Check '固定工具链' 'ERROR' ("固定 {0} 未安装（rustup home 中没有任何工具链）" -f $pin) "rustup toolchain install $pin --profile minimal --component rustfmt,clippy"
        }
        else {
            $pinnedFull = $toolchainNames | Where-Object { $_ -eq $pin -or $_ -like "$pin-*" } | Select-Object -First 1
            if ($pinnedFull) {
                Add-Check '固定工具链' 'OK' ("已安装: {0}（rustup home: {1}）" -f $pinnedFull, $rustupHome)
                $tcRoot = Join-Path $rustupHome ("toolchains\{0}" -f $pinnedFull)
                $rustcExe = Join-Path $tcRoot 'bin\rustc.exe'
                $cargoExe = Join-Path $tcRoot 'bin\cargo.exe'
                if (Test-Path -LiteralPath $rustcExe) {
                    try {
                        $v = (& $rustcExe --version 2>&1) -join ' '
                        $verMatch = [regex]::Match($v, '\d+\.\d+\.\d+')
                        if ($verMatch.Success) {
                            $ver = $verMatch.Value
                            if ($ver -eq $pin) {
                                Add-Check 'Rust 版本' 'OK' ("rustc {0}（{1}）" -f $ver, $v.Trim())
                            }
                            else {
                                Add-Check 'Rust 版本' 'WARN' ("固定 {0}，实际 rustc {1}" -f $pin, $ver)
                            }
                        }
                        else {
                            Add-Check 'Rust 版本' 'WARN' ("rustc 输出无法解析: {0}" -f $v)
                        }
                    }
                    catch {
                        Add-Check 'Rust 版本' 'ERROR' ("运行 rustc --version 失败: {0}" -f $_.Exception.Message)
                    }
                }
                else {
                    Add-Check 'Rust 版本' 'ERROR' ("工具链目录存在但找不到 {0}" -f $rustcExe) '重新安装该工具链'
                }
                if (Test-Path -LiteralPath $cargoExe) {
                    try {
                        $cv = (& $cargoExe --version 2>&1) -join ' '
                        Add-Check 'Cargo' 'INFO' ("cargo {0}" -f $cv.Trim())
                    }
                    catch {
                        Add-Check 'Cargo' 'WARN' ("cargo --version 失败: {0}" -f $_.Exception.Message)
                    }
                }
                # 目标与组件：直接读 rustup 管理的文件
                $rustLib = Join-Path $tcRoot 'lib\rustlib'
                if (Test-Path -LiteralPath (Join-Path $rustLib 'x86_64-pc-windows-msvc')) {
                    Add-Check 'MSVC 目标' 'OK' 'x86_64-pc-windows-msvc 已安装'
                }
                else {
                    Add-Check 'MSVC 目标' 'ERROR' '缺少 x86_64-pc-windows-msvc 目标' "rustup target add x86_64-pc-windows-msvc --toolchain $pinnedFull"
                }
                $componentsFile = Join-Path $rustLib 'components'
                if (Test-Path -LiteralPath $componentsFile) {
                    $comps = @(Get-Content -LiteralPath $componentsFile -ErrorAction SilentlyContinue)
                    $hasFmt = @($comps | Where-Object { $_ -like 'rustfmt*' }).Count -gt 0
                    $hasClippy = @($comps | Where-Object { $_ -like 'clippy*' }).Count -gt 0
                    if ($hasFmt -and $hasClippy) {
                        Add-Check 'rustfmt/clippy' 'OK' '两组件均已安装'
                    }
                    else {
                        $missing = @()
                        if (-not $hasFmt) { $missing += 'rustfmt' }
                        if (-not $hasClippy) { $missing += 'clippy' }
                        Add-Check 'rustfmt/clippy' 'ERROR' ("缺少: {0}" -f ($missing -join ', ')) "rustup component add $($missing -join ' ') --toolchain $pinnedFull"
                    }
                }
                else {
                    Add-Check 'rustfmt/clippy' 'WARN' '无法读取工具链组件清单（lib\rustlib\components 缺失）'
                }
            }
            else {
                Add-Check '固定工具链' 'ERROR' ("固定 {0} 未安装（现有: {1}）" -f $pin, ($toolchainNames -join ', ')) "rustup toolchain install $pin --profile minimal --component rustfmt,clippy"
            }
        }
    }
}

# ---------------------------------------------------------------- 3. MSVC / Windows SDK

function Test-DevShellLoaded {
    if ($env:VSCMD_VER) { return $true }
    if ($env:VCToolsInstallDir) { return $true }
    $cl = Get-Command cl.exe -ErrorAction SilentlyContinue
    if ($null -ne $cl -and $cl.Source -match '\\VC\\Tools\\MSVC\\') { return $true }
    return $false
}

$vs = Find-VisualStudio
if ($vs.Count -eq 0) {
    Add-Check 'MSVC / VS' 'ERROR' '未找到 Visual Studio Build Tools/Community 的 VC 工具' '用 VS Installer 安装"使用 C++ 的桌面开发"工作负载（含 MSVC 与 Windows SDK）'
    $msvcLoaded = $false
}
else {
    $best = $vs | Where-Object { Test-Path -LiteralPath $_.cl } | Select-Object -First 1
    if ($null -eq $best) { $best = $vs[0] }
    $clExists = Test-Path -LiteralPath $best.cl
    $vcvarsExists = Test-Path -LiteralPath $best.vcvars64
    $msvcLoaded = Test-DevShellLoaded
    if ($clExists) {
        $status = if ($msvcLoaded) { 'OK' } else { 'WARN' }
        $hint = if ($msvcLoaded) { '' } else { '未载入当前终端：运行 vcvars64.bat 或用 VS"x64 Native Tools"终端后重新执行' }
        Add-Check 'MSVC 编译器' $status ("版本 {0} @ {1}（来源: {2}）" -f $best.version, $best.cl, $best.source) $hint
        Add-Check 'MSVC 工具' 'INFO' ("link/rc: {0} / {1}" -f $best.link, $best.rc)
        if ($vcvarsExists) {
            Add-Check 'vcvars64.bat' 'INFO' $best.vcvars64
        }
    }
    else {
        Add-Check 'MSVC 编译器' 'ERROR' ("VC 工具目录存在但缺少 cl.exe: {0}" -f $best.cl) '检查 VS 安装完整性（VS Installer 修复）'
        $msvcLoaded = $false
    }
    if ($msvcLoaded -and -not $clExists) { $msvcLoaded = $false }
}

# Git 的 link.exe 干扰防护
$gitLink = Get-Command link.exe -ErrorAction SilentlyContinue
if ($null -ne $gitLink -and $gitLink.Source -match '\\Git\\') {
    Add-Check 'link.exe 假阳性' 'INFO' ("PATH 中检测到 Git 的 link.exe（非 MSVC）: {0}" -f $gitLink.Source)
}

if ($env:WindowsSdkDir) {
    $sdkVerShell = if ($env:WindowsSDKVersion) { $env:WindowsSDKVersion.TrimEnd('\') } else { '<版本未知>' }
    Add-Check 'Windows SDK' 'OK' ("Dev Shell 已提供: $env:WindowsSdkDir（$sdkVerShell）")
    $sdk = $null
}
else {
    $sdk = Get-WindowsSdk
    if ($null -ne $sdk) {
        $ok = (Test-Path -LiteralPath $sdk.rc) -and (Test-Path -LiteralPath $sdk.lib)
        if ($ok) {
            Add-Check 'Windows SDK' 'OK' ("SDK {0} @ {1}" -f $sdk.version, $sdk.root)
        }
        else {
            Add-Check 'Windows SDK' 'ERROR' ("SDK {0} 存在但缺少 rc.exe/kernel32.lib" -f $sdk.version) '用 VS Installer 修复 Windows SDK 组件'
        }
    }
    else {
        Add-Check 'Windows SDK' 'ERROR' '未找到 Windows SDK（Windows Kits\10）' '在 VS Installer 勾选 Windows SDK 组件后重开终端'
    }
}

# ---------------------------------------------------------------- 4. CMake

$cmake = Find-CMake
if ($null -ne $cmake) {
    try {
        $cmakeVer = ((& $cmake.path --version 2>&1) | Select-Object -First 1).Trim()
        if ($cmake.inPath) {
            Add-Check 'CMake' 'OK' ("{0}（PATH: {1}）" -f $cmakeVer, $cmake.path)
        }
        else {
            Add-Check 'CMake' 'WARN' ("已安装但不在 PATH: {0} @ {1}" -f $cmakeVer, $cmake.path) '手动调用该路径，或将包含 cmake.exe 的目录加入用户 PATH（需自行修改）'
        }
    }
    catch {
        Add-Check 'CMake' 'ERROR' ("cmake 无法执行: {0}" -f $_.Exception.Message)
    }
}
else {
    Add-Check 'CMake' 'ERROR' '未找到 cmake（PATH 与标准位置均无）' 'winget install Kitware.CMake；或在 VS Installer 勾选"C++ CMake tools for Windows"组件；完成后重开终端'
}

# ---------------------------------------------------------------- 5. 关键环境变量

$envInfo = @()
if ($env:VSCMD_VER)    { $envInfo += "VSCMD_VER=$($env:VSCMD_VER)" }
if ($env:VCToolsInstallDir) { $envInfo += "VCToolsInstallDir=$($env:VCToolsInstallDir)" }
if ($env:WindowsSdkDir) { $envInfo += "WindowsSdkDir=$($env:WindowsSdkDir)" }
if ($env:RUSTUP_HOME)  { $envInfo += "RUSTUP_HOME=$($env:RUSTUP_HOME)" }
if ($env:CARGO_HOME)   { $envInfo += "CARGO_HOME=$($env:CARGO_HOME)" }
if ($envInfo.Count -gt 0) {
    Add-Check '关键环境变量' 'INFO' ($envInfo -join '；')
}
else {
    Add-Check '关键环境变量' 'INFO' 'VSCMD_VER/VCToolsInstallDir/WindowsSdkDir 均未设置（普通 shell）'
}
$cargoOnPath = Get-Command cargo -ErrorAction SilentlyContinue
if ($null -eq $cargoOnPath) { Add-Check 'PATH: cargo' 'INFO' 'cargo 不在 PATH（会从 rustup home 定位）' }

# ---------------------------------------------------------------- 6. MSVC 空程序冒烟

if (-not $SkipCompile) {
    if ($vs.Count -gt 0 -and $msvcLoaded) {
        $tmp = Join-Path ([IO.Path]::GetTempPath()) ("omw-doctor-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-Item -ItemType Directory -Path $tmp -Force | Out-Null
            $src = Join-Path $tmp 'smoke.cpp'
            Set-Content -Encoding ascii -LiteralPath $src -Value 'int main(void) { return 0; }'
            # 相对文件名（当前目录 = $tmp）避开 PowerShell 原生参数引号被 cl 拆分的问题
            Push-Location $tmp
            try {
                $clOut = & $best.cl /nologo /EHsc smoke.cpp /Fosmoke.obj /Fesmoke.exe 2>&1
                $rc = $LASTEXITCODE
            }
            finally {
                Pop-Location
            }
            if ($rc -eq 0 -and (Test-Path -LiteralPath (Join-Path $tmp 'smoke.exe'))) {
                Add-Check 'MSVC 空程序编译' 'OK' ("cl {0} 编译链接成功（smoke.exe）" -f $best.version)
            }
            else {
                $diag = ($clOut | Select-Object -First 5) -join ' '
                Add-Check 'MSVC 空程序编译' 'ERROR' ("cl 退出码 {0}: {1}" -f $rc, $diag) '检查 Dev Shell 完整性（INCLUDE/LIB 是否设置）'
            }
        }
        catch {
            Add-Check 'MSVC 空程序编译' 'ERROR' ("异常: {0}" -f $_.Exception.Message)
        }
        finally {
            Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    else {
        Add-Check 'MSVC 空程序编译' 'INFO' '跳过：MSVC 未载入当前 shell（请先进入 VS Developer 环境）'
    }
}
else {
    Add-Check 'MSVC 空程序编译' 'INFO' '跳过（-SkipCompile）'
}

# ---------------------------------------------------------------- 汇总与退出码

$okCount    = @($script:checks | Where-Object { $_.status -eq 'OK' }).Count
$warnCount  = @($script:checks | Where-Object { $_.status -eq 'WARN' }).Count
$errorCount = @($script:checks | Where-Object { $_.status -eq 'ERROR' }).Count
$infoCount  = @($script:checks | Where-Object { $_.status -eq 'INFO' }).Count

$exitCode = if ($errorCount -gt 0) { 2 } elseif ($warnCount -gt 0) { 1 } else { 0 }

if ($Json) {
    [pscustomobject]@{
        repo  = $repoRoot
        exit  = $exitCode
        ok    = $okCount
        warn  = $warnCount
        error = $errorCount
        info  = $infoCount
        checks = $script:checks
    } | ConvertTo-Json -Depth 6
}
else {
    Write-Host ''
    Write-Host ("结果: {0} OK / {1} WARN / {2} ERROR / {3} INFO → 退出码 {4}" -f $okCount, $warnCount, $errorCount, $infoCount, $exitCode)
}
exit $exitCode