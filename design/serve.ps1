# Serves this folder on http://localhost:5178 for previewing design prototypes.
$root = $PSScriptRoot
$l = New-Object Net.HttpListener
$l.Prefixes.Add('http://localhost:5178/')
$l.Start()
Write-Host 'serving on http://localhost:5178'
while ($l.IsListening) {
    $c = $l.GetContext()
    $name = [Uri]::UnescapeDataString($c.Request.Url.AbsolutePath.TrimStart('/'))
    if (-not $name) { $name = 'hum-prototype.html' }
    $path = Join-Path $root $name
    if ((Test-Path $path -PathType Leaf) -and ([IO.Path]::GetFullPath($path).StartsWith($root))) {
        $bytes = [IO.File]::ReadAllBytes($path)
        $c.Response.ContentType = if ($path -like '*.html') { 'text/html; charset=utf-8' } else { 'application/octet-stream' }
        $c.Response.OutputStream.Write($bytes, 0, $bytes.Length)
    } else { $c.Response.StatusCode = 404 }
    $c.Response.Close()
}
