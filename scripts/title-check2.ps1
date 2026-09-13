# 精细化标题比对 v2：报告所有"本地≠网站"的差异，并分类
# 分类：标题不一致 / 仅演员名或附加文字差异 / 网站未找到

function Read-JsonFile($path) {
  $raw = [System.IO.File]::ReadAllText($path)
  $raw = $raw.TrimStart([char]0xFEFF)
  return $raw | ConvertFrom-Json
}

$local = Read-JsonFile "C:\Users\LinYanZhi\Code\lzy-leisure\scripts\local-files.json"
$site = Read-JsonFile "C:\Users\LinYanZhi\Code\lzy-leisure\scripts\site-titles.json"

$siteMap = @{}
foreach ($p in $site.PSObject.Properties) { $siteMap[$p.Name] = $p.Value }
$plateFix = @{ 'PZZ-655' = 'IPZZ-655' }

function Normalize-Title($text, $plate) {
  $s = $text
  if ($plate) { $s = $s -replace [regex]::Escape($plate), '' }
  $s = $s -replace '\[中字\]', ''
  $s = $s -replace '\(中字\)', ''
  $s = $s -replace '\(中字', ''
  $s = $s -replace '\(\d+\)', ''
  $s = $s -replace '\d{3,4}p', ''
  $s = $s -replace 'x264|x265|hevc|av1|bluray|webrip|dvdrip|10bit', ''
  $s = $s -replace '\.(mp4|mkv|avi|wmv|flv|rmvb|ts|webm|mov)$', ''
  $s = ($s -replace '\s', '').ToLowerInvariant()
  return $s
}

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

# 已知演员名/别名（用于识别"仅演员名差异"）
$actorNames = @(
  '枫可怜','楓カレン','枫花恋','枫卡伦','凯伦枫','田中柠檬','田中レモン','karenkaede',
  '明里紬','明里䌷','明里つむぎ','akari',
  '新有菜','桥本有菜','新ありな','arata','arinahashimoto',
  '三好佑香','三好由香','みよし','miyoshi',
  '天使萌','天使もえ','tenshi',
  '三上悠亚','三上悠亜','yua','mikami',
  '桃乃木香奈','桃乃木かな','桃野木佳奈','momonoki',
  '河北彩花','河北彩伽','川北彩香','川北彩夏','川北绫香','kawakita',
  '凪ひかる','凪ひかり','凪光','汐世','有栖花あか','有栖花绯','nagi',
  '滨崎真绪','浜崎真緒','hamasaki',
  '本庄铃','本庄鈴','本城凛','honjo','suzu',
  '森泽佳奈','森沢かな','饭冈佳奈子','飯岡かなこ','moriwaka','森泽',
  '樱空桃','桜空もも','佐仓桃桃','sakura','momo',
  '楪可怜','楪カレン','柚叶夏莲','yuzuriha',
  '枫芙爱','楓ふうあ','风华枫','kaede','fua',
  '山岸逢花','山岸あや花','山岸绫香','yamagishi',
  '向井蓝','向井藍','mukai',
  '七濑爱丽丝','七瀬アリス','nanase',
  '仲村美宇','仲村美优','仲村みう','仲村美羽','nakamura','miu',
  '鹫尾芽衣','鷲尾めい','washio',
  '新井优香','新井優香','arai',
  '神木丽','上木丽','神宫寺奈绪','小岛南','うんぱい','白桃花','石川澪','宫下玲奈','月云よる','和久井美兔','梦乃爱佳','梦乃爱华','彩月七绪','紫堂るい','志堂瑠衣','村上悠华','希岛爱里','佐々木さき','佐佐木沙希','芦田希空','新井リマ','新井莉玛','かにもちゃり','筧ジュン','彩月','希空'
)

$rows = @()
foreach ($l in $local) {
  $plate = $l.plate
  $fileName = $l.file

  $sitePlate = $plate
  $plateMismatch = $false
  if (-not $siteMap.ContainsKey($plate)) {
    if ($plateFix.ContainsKey($plate) -and $siteMap.ContainsKey($plateFix[$plate])) {
      $sitePlate = $plateFix[$plate]; $plateMismatch = $true
    } else {
      $rows += [PSCustomObject]@{ 车牌=$plate; 分类='网站未找到'; 相似度=0; 本地=$fileName; 网站='' }
      continue
    }
  }
  $siteTitle = $siteMap[$sitePlate]
  $localNorm = Normalize-Title $fileName $plate
  $siteNorm  = Normalize-Title $siteTitle $sitePlate
  $sim = [math]::Round((Similarity $localNorm $siteNorm), 3)

  if ($plateMismatch) {
    $rows += [PSCustomObject]@{ 车牌=$plate; 分类=('车牌错误(应为'+$sitePlate+')'); 相似度=$sim; 本地=$fileName; 网站=$siteTitle }
    continue
  }

  # 完全一致 → 跳过
  if ($localNorm -eq $siteNorm) { continue }

  # 去掉演员名后再比：若一致 → 仅演员名差异
  $localNoActor = $localNorm
  $siteNoActor = $siteNorm
  foreach ($a in $actorNames) {
    $localNoActor = $localNoActor.Replace($a, '')
    $siteNoActor  = $siteNoActor.Replace($a, '')
  }
  $localNoActor = $localNoActor -replace '[-_—－·]', ''
  $siteNoActor  = $siteNoActor -replace '[-_—－·]', ''

  if ($localNoActor -eq $siteNoActor -and $localNoActor.Length -gt 2) {
    $rows += [PSCustomObject]@{ 车牌=$plate; 分类='仅演员名前缀差异'; 相似度=$sim; 本地=$fileName; 网站=$siteTitle }
  } elseif ($sim -lt 0.6) {
    $rows += [PSCustomObject]@{ 车牌=$plate; 分类='标题不一致'; 相似度=$sim; 本地=$fileName; 网站=$siteTitle }
  } else {
    # 本地是否 = [演员名(中字)]前缀 + 网站标题？
    $m = [regex]::Match($localNorm, '^\[[^\]]*\]')
    if ($m.Success) {
      $afterPrefix = $localNorm.Substring($m.Length)
      $afterPrefix = ($afterPrefix -replace '[-_—－·]', '')
      $siteClean = ($siteNorm -replace '[-_—－·]', '')
      if ($afterPrefix -eq $siteClean -or ($siteClean.Length -gt $afterPrefix.Length -and $siteClean.StartsWith($afterPrefix))) {
        $rows += [PSCustomObject]@{ 车牌=$plate; 分类='仅本地带演员名前缀'; 相似度=$sim; 本地=$fileName; 网站=$siteTitle }
        continue
      }
    }
    $rows += [PSCustomObject]@{ 车牌=$plate; 分类='文本有差异(演员名/措辞)'; 相似度=$sim; 本地=$fileName; 网站=$siteTitle }
  }
}

Write-Output ("总检查 {0} 条 | 有差异 {1} 条" -f $local.Count, $rows.Count)
Write-Output ""
Write-Output "=== 分类汇总 ==="
$rows | Group-Object 分类 | Select-Object Name, Count | Format-Table -AutoSize
Write-Output ""
Write-Output "=== 明细 ==="
foreach ($r in ($rows | Sort-Object 分类, 相似度)) {
  Write-Output ("[{0}] {1} (相似度 {2})" -f $r.车牌, $r.分类, $r.相似度)
  Write-Output ("  本地: {0}" -f $r.本地)
  if ($r.网站) { Write-Output ("  网站: {0}" -f $r.网站) }
  Write-Output ""
}
$rows | Export-Csv "D:\视频\cat-catch\标题比对结果.csv" -Encoding UTF8 -NoTypeInformation
Write-Output "已导出: D:\视频\cat-catch\标题比对结果.csv"