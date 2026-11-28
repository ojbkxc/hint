#!/usr/bin/env pwsh
<#
.SYNOPSIS
    语音识别自检：用系统 TTS 合成中文/英文语音，喂给 hint 的 voice worker 识别，比对结果。

.DESCRIPTION
    用途：验证「本地语音识别 → worker → 识别结果」这条链路是否正常，
    以及验证最近改动（如去直流高通）有没有破坏识别。

    ⚠️ 为什么必须这样写（血的教训）：

    **中文文本绝对不能通过 stdin 传给 PowerShell。**
    实测过：同一个字符串
      - 内嵌在命令里 → 合成音频正确 → 识别 "大家好，我是候选人。"
      - 从 stdin 传   → UTF-8 字节被按 GBK 解释 → 合成音频完全是另一段声音
                        （与正确音频的相关系数 -0.0000）→ 识别出一串乱码
    这份乱码曾让人误判「产品中文识别坏了」，白查十几轮（模型、VAD、重采样、
    噪声门、音效、采样率全都排查过，实际都是脚本自己的锅）。

    所以本脚本把中文文本**写死在代码里**，绝不经 stdin。

.PARAMETER ExePath
    hint.exe 的路径。默认取安装目录。

.PARAMETER DumpDir
    若指定，把每段合成音频另存到该目录（便于人工试听核对）。

.EXAMPLE
    pwsh scripts/verify-stt.ps1
    pwsh scripts/verify-stt.ps1 -DumpDir D:\tmp\stt-dump
#>
[CmdletBinding()]
param(
    [string]$ExePath = 'D:\UniAccessAgentDownloadData\AppStore\hint\hint.exe',
    [string]$DumpDir = ''
)

$ErrorActionPreference = 'Stop'

function Write-Step([string]$Message) {
    Write-Host "`n=== $Message ===" -ForegroundColor Cyan
}

function Write-Pass([string]$Message) {
    Write-Host "PASS  $Message" -ForegroundColor Green
}

function Write-Fail([string]$Message) {
    Write-Host "FAIL  $Message" -ForegroundColor Red
}

# ---------------------------------------------------------------- 前置检查

if (-not (Test-Path $ExePath)) {
    Write-Fail "找不到 hint.exe：$ExePath"
    exit 2
}

$modelDir = Join-Path $env:LOCALAPPDATA 'hint-models\sensevoice\models'
if (-not (Test-Path $modelDir)) {
    Write-Fail "找不到语音模型目录：$modelDir（先在设置页下载模型）"
    exit 2
}

function Find-Model([string]$Name) {
    $hit = Get-ChildItem -Path $modelDir -Recurse -Filter $Name -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($hit) { return $hit.FullName }
    return $null
}

$modelPath = (Find-Model 'model.int8.onnx')
if (-not $modelPath) { $modelPath = (Find-Model 'model.onnx') }
$tokensPath = Find-Model 'tokens.txt'
$vadPath = Find-Model 'silero_vad.onnx'

if (-not $modelPath -or -not $tokensPath -or -not $vadPath) {
    Write-Fail "模型文件不全（model/tokens/vad）：$modelDir"
    exit 2
}

Write-Host "模型目录: $modelDir"

# ---------------------------------------------------------------- 生成测试音频
#
# 关键：中文文本内嵌在下面的字符串里，不经 stdin、不经参数传递。

$cases = @(
    @{ Name = 'zh-short'; Text = '大家好，我是候选人。'; Expect = '大家好，我是候选人' }
    @{ Name = 'zh-long';  Text = '我在上一个项目里负责自动化测试工作。'; Expect = '我在上一个项目里负责自动化测试' }
    @{ Name = 'zh-mixed'; Text = '主要使用 Playwright 和 Python 做接口测试。'; Expect = '做接口测试' }
    # 语气词 + 长句：真实面试里说话不会是工整的单句，会有「嗯」「那个」这类填充。
    # 验证填充词不会把后半句吞掉，也不该被单独切成一段噪声。
    @{ Name = 'zh-filler'; Text = '嗯，那个，我在上一家公司主要做的是自动化测试。'; Expect = '自动化测试' }
    # 停顿长句：一句话中间有明显停顿，验证断句后内容仍完整（不因停顿丢字）。
    @{ Name = 'zh-paused'; Text = '我负责过，两个大型项目，的自动化测试工作。'; Expect = '自动化测试' }
)

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("hint-stt-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

# ---------------------------------------------------------------- 规范化 WAV
#
# System.Speech 写出的 WAV 头部不是规范 44 字节（带额外的 LIST/fact chunk 等），
# 而 worker 会严格校验并拒绝非规范头：
#   {"error":"audio is not a canonical 44-byte-header WAV file","code":"invalid-input"}
# 产品的实际路径是自己 encode_wav 生成规范头，所以不受影响；测试脚本要补这一步。
function Convert-ToCanonicalWav([string]$InPath, [string]$OutPath) {
    $bytes = [System.IO.File]::ReadAllBytes($InPath)

    # 解析 RIFF/WAVE，找到 fmt 与 data chunk（不假定固定偏移）
    if ($bytes.Length -lt 44) { throw "WAV 太短：$InPath" }
    $riff = [System.Text.Encoding]::ASCII.GetString($bytes, 0, 4)
    $wave = [System.Text.Encoding]::ASCII.GetString($bytes, 8, 4)
    if ($riff -ne 'RIFF' -or $wave -ne 'WAVE') { throw "不是 RIFF/WAVE：$InPath" }

    $pos = 12
    $fmtData = $null
    $audioData = $null
    while ($pos + 8 -le $bytes.Length) {
        $id = [System.Text.Encoding]::ASCII.GetString($bytes, $pos, 4)
        $size = [BitConverter]::ToInt32($bytes, $pos + 4)
        $body = $pos + 8
        if ($id -eq 'fmt ') {
            $fmtData = New-Object byte[] $size
            [Array]::Copy($bytes, $body, $fmtData, 0, [Math]::Min($size, $bytes.Length - $body))
        } elseif ($id -eq 'data') {
            $avail = $bytes.Length - $body
            $n = [Math]::Min($size, $avail)
            $audioData = New-Object byte[] $n
            [Array]::Copy($bytes, $body, $audioData, 0, $n)
            break
        }
        # chunk 按偶数字节对齐
        $pos = $body + $size + ($size % 2)
    }

    if ($null -eq $fmtData) { throw "WAV 缺少 fmt chunk：$InPath" }
    if ($null -eq $audioData) { throw "WAV 缺少 data chunk：$InPath" }

    # 写规范 44 字节头：RIFF + size + WAVE + "fmt " + 16 + PCM 头 + "data" + size
    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter($ms)
    $bw.Write([char[]]'RIFF')
    $bw.Write([int](36 + $audioData.Length))
    $bw.Write([char[]]'WAVE')
    $bw.Write([char[]]'fmt ')
    $bw.Write([int]16)
    $bw.Write($fmtData, 0, [Math]::Min(16, $fmtData.Length))
    $bw.Write([char[]]'data')
    $bw.Write([int]$audioData.Length)
    $bw.Write($audioData)
    $bw.Flush()
    [System.IO.File]::WriteAllBytes($OutPath, $ms.ToArray())
    $bw.Close()
    $ms.Close()
}

Write-Step '合成测试语音（16kHz 单声道）'

$made = @()
foreach ($case in $cases) {
    $wav = Join-Path $tempDir ($case.Name + '.wav')
    # 用 -Command 传整个脚本，中文因此留在命令文本里（不是 stdin）
    $ps = @"
Add-Type -AssemblyName System.Speech
`$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
`$s.SelectVoice('Microsoft Huihui Desktop')
`$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
`$s.SetOutputToWaveFile('$wav', `$fmt)
`$s.Speak('$($case.Text)')
`$s.Dispose()
"@
    & powershell -NoProfile -Command $ps
    if (-not (Test-Path $wav)) {
        Write-Fail "合成失败：$($case.Name)"
        continue
    }
    # 规范化头部（worker 只接受 44 字节规范头）
    $canonical = Join-Path $tempDir ($case.Name + '-c.wav')
    try {
        Convert-ToCanonicalWav -InPath $wav -OutPath $canonical
        Remove-Item $wav -Force
        Move-Item $canonical $wav -Force
    } catch {
        Write-Fail "规范化 WAV 失败（$($case.Name)）：$($_.Exception.Message)"
        continue
    }
    Write-Host ("  {0,-10} {1}" -f $case.Name, $case.Text)
    $made += [pscustomobject]@{ Case = $case; Path = $wav }
}

if ($made.Count -eq 0) {
    Write-Fail '没有任何测试音频生成成功'
    Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

# ---------------------------------------------------------------- 启动 worker

Write-Step '启动 voice worker'

$config = @{
    model_path      = $modelPath
    tokens_path     = $tokensPath
    vad_path        = $vadPath
    threads         = 2
    max_audio_bytes = 20 * 1024 * 1024
    max_audio_seconds = 60.0
} | ConvertTo-Json -Compress

$token = 'verify' + [guid]::NewGuid().ToString('N')

$env:HINT_VOICE_WORKER_CONFIG = $config
$env:HINT_VOICE_WORKER_TOKEN = $token

# 注意：stdin 必须保持打开 —— worker 的生命周期绑定 stdin EOF，
# 若立即 EOF，它会启动后马上退出，表现为「connection reset」。
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $ExePath
$psi.Arguments = '--voice-worker'
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.EnvironmentVariables['HINT_VOICE_WORKER_CONFIG'] = $config
$psi.EnvironmentVariables['HINT_VOICE_WORKER_TOKEN'] = $token

$worker = [System.Diagnostics.Process]::Start($psi)
$port = $null
$deadline = (Get-Date).AddSeconds(60)
while ((Get-Date) -lt $deadline) {
    $line = $worker.StandardOutput.ReadLine()
    if ($null -eq $line) { break }
    if ($line -match '"port"\s*:\s*(\d+)') {
        $port = [int]$Matches[1]
        break
    }
}

if (-not $port) {
    Write-Fail 'worker 未能启动（未读到端口）'
    Write-Host $worker.StandardError.ReadToEnd()
    try { $worker.Kill() } catch {}
    Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

Write-Host "  port = $port"

# ---------------------------------------------------------------- 识别并比对

function Invoke-Transcribe([string]$WavPath, [int]$Port, [string]$Token) {
    # 用 curl.exe（Windows 10+ 自带）：.NET 的 HttpWebRequest 在这台机器上会被
    # worker 关闭连接（"connection was closed unexpectedly"），curl 走裸 HTTP/1.1
    # 反而稳。响应是 JSON，用 ConvertFrom-Json 解析。
    $url = "http://127.0.0.1:$Port/transcribe?language=zh"
    $raw = & curl.exe -s -X POST `
        -H "Authorization: Bearer $Token" `
        -H 'Content-Type: audio/wav' `
        --data-binary "@$WavPath" `
        $url 2>&1

    if ($LASTEXITCODE -ne 0) {
        throw "curl 退出码 $LASTEXITCODE：$raw"
    }
    $text = ($raw | Out-String).Trim()
    if (-not $text) {
        throw '空响应'
    }
    return ($text | ConvertFrom-Json)
}

Write-Step '识别比对'

$pass = 0
$fail = 0
foreach ($item in $made) {
    try {
        $result = Invoke-Transcribe -WavPath $item.Path -Port $port -Token $token
        $text = ($result.text, $result.Text | Where-Object { $_ } | Select-Object -First 1)
        if (-not $text) { $text = '' }
        $ok = $text -like ('*' + $item.Case.Expect + '*')
        if ($ok) {
            Write-Pass ("{0,-10} -> {1}" -f $item.Case.Name, $text)
            $pass++
        } else {
            Write-Fail ("{0,-10} -> {1}   (期望包含：{2})" -f $item.Case.Name, $text, $item.Case.Expect)
            $fail++
        }
    } catch {
        Write-Fail ("{0,-10} 识别请求失败：{1}" -f $item.Case.Name, $_.Exception.Message)
        $fail++
    }
}

# ---------------------------------------------------------------- 收尾

try { $worker.Kill() } catch {}

if ($DumpDir) {
    New-Item -ItemType Directory -Path $DumpDir -Force | Out-Null
    Copy-Item (Join-Path $tempDir '*.wav') $DumpDir -Force
    Write-Host "`n音频已保留到：$DumpDir"
}

Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item Env:\HINT_VOICE_WORKER_CONFIG -ErrorAction SilentlyContinue
Remove-Item Env:\HINT_VOICE_WORKER_TOKEN -ErrorAction SilentlyContinue

Write-Step "结果：$pass 通过 / $fail 失败"

if ($fail -gt 0) {
    Write-Host @'

排查提示：
  - 先确认中文文本是**内嵌**在命令里的（本脚本就是这么做的），
    用 stdin 传中文会被按 GBK 解释，合成出完全不同的音频。
  - 若识别为空或乱码，用 -DumpDir 保留音频，人工试听确认是不是合成问题。
  - 真实采集链路（WASAPI loopback / 麦克风）与合成音频是两条路径，
    合成通过不代表真实采集通过；后者要用 HINT_DUMP_STT 抓真实音频。
'@
    exit 1
}

exit 0
