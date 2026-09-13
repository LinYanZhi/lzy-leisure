# 对比本地视频文件名 vs 网站标题，找出标题不一致的车牌
# 用法：node 需要？不，纯 PowerShell

# ── 读取 JSON（容忍 BOM） ──
function Read-JsonFile($path) {
  $raw = [System.IO.File]::ReadAllText($path)
  $raw = $raw.TrimStart([char]0xFEFF)
  return $raw | ConvertFrom-Json
}

$local = Read-JsonFile "C:\Users\LinYanZhi\Code\lzy-leisure\scripts\local-files.json"
$site = Read-JsonFile "C:\Users\LinYanZhi\Code\lzy-leisure\scripts\site-titles.json"

# 建 site 查找表
$siteMap = @{}
foreach ($p in $site.PSObject.Properties) { $siteMap[$p.Name] = $p.Value }

# 车牌错误修正映射（本地笔误 → 正确车牌）
$plateFix = @{ 'PZZ-655' = 'IPZZ-655' }

# ── 规范化：去掉车牌、空白、杂质标签 ──
function Normalize-Title($text, $plate) {
  $s = $text
  # 去掉车牌号前缀（可能是 "IPX-564 " 或 "IPX-564"）
  if ($plate) { $s = $s -replace [regex]::Escape($plate), '' }
  # 去掉杂质标签（只去括号包裹的"中字"等，避免误伤"中出/中字幕"等正常词）
  $s = $s -replace '\[中字\]', ''
  $s = $s -replace '\(中字\)', ''
  $s = $s -replace '\(中字', ''
  $s = $s -replace '\(\d+\)', ''
  $s = $s -replace '\d{3,4}p', ''
  $s = $s -replace 'x264|x265|hevc|av1|bluray|webrip|dvdrip|10bit', ''
  $s = $s -replace '\.(mp4|mkv|avi|wmv|flv|rmvb|ts|webm|mov)$', ''
  # 去所有空白
  $s = ($s -replace '\s', '').ToLowerInvariant()
  return $s
}

# ── 字符 bigram 相似度（0-1） ──
function Similarity($a, $b) {
  if ($a.Length -eq 0 -or $b.Length -eq 0) { return 0 }
  function Bigrams($s) {
    $set = New-Object 'System.Collections.Generic.HashSet[string]'
    for ($i = 0; $i -lt $s.Length - 1; $i++) { [void]$set.Add($s.Substring($i, 2)) }
    return $set
  }
  $A = Bigrams $a
  $B = Bigrams $b
  $inter = 0
  foreach ($x in $A) { if ($B.Contains($x)) { $inter++ } }
  $union = $A.Count + $B.Count - $inter
  if ($union -eq 0) { return 0 }
  return $inter / $union
}

$issues = @()
$checked = 0

foreach ($l in $local) {
  $plate = $l.plate
  $fileName = $l.file
  $checked++

  # 1. 网站是否存在（考虑车牌修正）
  $sitePlate = $plate
  $plateMismatch = $false
  if (-not $siteMap.ContainsKey($plate)) {
    if ($plateFix.ContainsKey($plate) -and $siteMap.ContainsKey($plateFix[$plate])) {
      $sitePlate = $plateFix[$plate]
      $plateMismatch = $true
    } else {
      # 真不在网站
      $issues += [PSCustomObject]@{ 车牌 = $plate; 类型 = '网站未找到'; 本地文件 = $fileName; 相似度 = 0; 网站标题 = '' }
      continue
    }
  }
  $siteTitle = $siteMap[$sitePlate]

  # 2. 规范化 + 相似度
  $localNorm = Normalize-Title $fileName $plate
  $siteNorm = Normalize-Title $siteTitle $sitePlate
  $sim = [math]::Round((Similarity $localNorm $siteNorm), 3)

  # 3. 判定：相似度过低 = 标题不一致；车牌错误单独标记
  if ($plateMismatch) {
    $issues += [PSCustomObject]@{ 车牌 = $plate; 类型 = '车牌错误(应为' + $sitePlate + ')'; 本地文件 = $fileName; 相似度 = $sim; 网站标题 = $siteTitle }
  } elseif ($sim -lt 0.6) {
    $issues += [PSCustomObject]@{ 车牌 = $plate; 类型 = '标题不一致'; 本地文件 = $fileName; 相似度 = $sim; 网站标题 = $siteTitle }
  }
}

Write-Output ("已检查: {0} 条 | 发现异常: {1} 条" -f $checked, $issues.Count)
Write-Output ""
$issues | Format-Table 车牌, 类型, 相似度 -AutoSize | Out-String -Width 120 | Write-Output
Write-Output "=== 详细清单 ==="
foreach ($i in $issues) {
  Write-Output ("[{0}] {1} (相似度 {2})" -f $i.车牌, $i.类型, $i.相似度)
  Write-Output ("  本地: {0}" -f $i.本地文件)
  if ($i.网站标题) { Write-Output ("  网站: {0}" -f $i.网站标题) }
  Write-Output ""
}

# 输出到文件
$issues | Export-Csv "D:\视频\cat-catch\标题比对结果.csv" -Encoding UTF8 -NoTypeInformation
Write-Output "已导出: D:\视频\cat-catch\标题比对结果.csv"