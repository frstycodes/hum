# Builds Hum for Android without Gradle: aapt2 (resources), javac, d8 (dex),
# zipalign and apksigner, straight from the SDK. Output: android\out\hum.apk
# Tools: %LOCALAPPDATA%\AndroidBuild (JDK 17, SDK platform 35, build-tools 35.0.0),
# or override with -Sdk / -Jdk.
param(
    [string]$Sdk = "$env:LOCALAPPDATA\AndroidBuild\sdk",
    [string]$Jdk = (Get-ChildItem "$env:LOCALAPPDATA\AndroidBuild\jdk" -Directory -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$bt = "$Sdk\build-tools\35.0.0"
$jar = "$Sdk\platforms\android-35\android.jar"
foreach ($p in $bt, $jar, $Jdk) { if (-not (Test-Path $p)) { throw "missing: $p" } }
$env:JAVA_HOME = $Jdk
$env:PATH = "$Jdk\bin;$env:PATH"

$out = "$here\out"
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$out\res", "$out\gen", "$out\classes", "$out\dex", "$out\assets" | Out-Null
Copy-Item "$here\..\assets\DMSans.ttf" "$out\assets\DMSans.ttf"

function Run($exe) {
    # tools chat on stderr; only the exit code means failure
    $ErrorActionPreference = 'Continue'
    & $exe @args 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw "$exe failed ($LASTEXITCODE)" }
}

# resources -> base APK + R.java
Run "$bt\aapt2.exe" compile --dir "$here\res" -o "$out\res\res.zip"
Run "$bt\aapt2.exe" link -o "$out\base.apk" -I $jar --manifest "$here\AndroidManifest.xml" `
    --java "$out\gen" -A "$out\assets" --min-sdk-version 26 --target-sdk-version 35 "$out\res\res.zip"

# Java -> classes -> dex
$sources = @(Get-ChildItem "$here\src", "$out\gen" -Recurse -Filter *.java | ForEach-Object FullName)
Run "$Jdk\bin\javac.exe" --release 11 '-Xlint:-options' -encoding UTF-8 -classpath $jar -d "$out\classes" @sources
$classes = @(Get-ChildItem "$out\classes" -Recurse -Filter *.class | ForEach-Object FullName)
Run "$bt\d8.bat" --release --min-api 26 --lib $jar --output "$out\dex" @classes

# dex into the APK, align, sign
Copy-Item "$out\base.apk" "$out\unsigned.apk"
Push-Location "$out\dex"
try { Run "$Jdk\bin\jar.exe" uf "$out\unsigned.apk" classes.dex } finally { Pop-Location }
Run "$bt\zipalign.exe" -f -p 4 "$out\unsigned.apk" "$out\aligned.apk"

# A local signing key, made once and kept beside the project (updates must reuse it).
# Its password comes from HUM_KEYSTORE_PASS or android\keystore.pass; neither the key
# nor the password is ever committed (.gitignore).
$ks = "$here\hum-release.keystore"
$passFile = "$here\keystore.pass"
$pass = $env:HUM_KEYSTORE_PASS
if (-not $pass -and (Test-Path $passFile)) { $pass = (Get-Content $passFile -Raw).Trim() }
if (-not (Test-Path $ks)) {
    if (-not $pass) {
        $bytes = New-Object byte[] 18
        [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        $pass = [Convert]::ToBase64String($bytes).Replace('+', 'x').Replace('/', 'y').Replace('=', '')
        [IO.File]::WriteAllText($passFile, $pass)
    }
    Run "$Jdk\bin\keytool.exe" -genkeypair -keystore $ks -storepass $pass -keypass $pass -alias hum `
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Hum"
}
if (-not $pass) { throw "the signing key's password: set HUM_KEYSTORE_PASS or put it in $passFile" }
Run "$bt\apksigner.bat" sign --ks $ks --ks-pass "pass:$pass" --key-pass "pass:$pass" --out "$out\hum.apk" "$out\aligned.apk"
Run "$bt\apksigner.bat" verify "$out\hum.apk"
Get-Item "$out\hum.apk"
