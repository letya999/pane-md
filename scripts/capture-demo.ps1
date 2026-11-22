param(
    [string]$Document = 'examples/demo.md',
    [string]$Output = 'docs/media/pane-md-demo.png'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$documentPath = (Resolve-Path (Join-Path $root $Document)).Path
$outputPath = Join-Path $root $Output
New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
$launcher = Join-Path $env:LOCALAPPDATA 'Programs\pane-md\pane-md-editor.exe'
if (-not (Test-Path -LiteralPath $launcher)) { throw "Install pane-md before capturing: $launcher" }

Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DemoWindow {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hWnd, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
}
'@
[DemoWindow]::SetProcessDPIAware() | Out-Null

$settingsKey = 'HKCU:\Software\pane-md'
$settings = Get-ItemProperty $settingsKey
$originalTheme = $settings.Theme
$originalAppearance = $settings.EditorAppearance
Set-ItemProperty $settingsKey Theme 184 # Dark theme for the published screenshot.
if ($originalAppearance) {
    $appearance = ConvertFrom-Json -AsHashtable $originalAppearance
    $appearance.rich.half_points = 25
    Set-ItemProperty $settingsKey EditorAppearance (ConvertTo-Json -Compress -Depth 8 $appearance)
}

try {

Get-Process pane-md -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowTitle -like 'demo.md * pane-md' } |
    ForEach-Object { $_.CloseMainWindow() | Out-Null; $_.WaitForExit(5000) | Out-Null }

Start-Process -FilePath $launcher -ArgumentList ('--open "' + $documentPath + '"') | Out-Null
$window = $null
for ($attempt = 0; $attempt -lt 40; $attempt++) {
    $window = Get-Process pane-md -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowTitle -like 'demo.md * pane-md' } |
        Select-Object -First 1
    if ($window) { break }
    Start-Sleep -Milliseconds 250
}
if (-not $window) { throw 'Demo window did not open' }
$handle = $window.MainWindowHandle
[DemoWindow]::ShowWindow($handle, 9) | Out-Null
[DemoWindow]::SetWindowPos($handle, [IntPtr]::Zero, 45, 45, 1650, 900, 0x40) | Out-Null
[DemoWindow]::SetForegroundWindow($handle) | Out-Null

function Invoke-CommandId([int]$id) {
    [DemoWindow]::SendMessage($handle, 0x111, [UIntPtr]::new([uint32]$id), [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 650
}

function Save-Frame {
    [DemoWindow]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 350
    $rect = New-Object DemoWindow+Rect
    [DemoWindow]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $hdc = $graphics.GetHdc()
        try { $rendered = [DemoWindow]::PrintWindow($handle, $hdc, 2) }
        finally { $graphics.ReleaseHdc($hdc) }
        if (-not $rendered) { $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size) }
        $bitmap.Save($outputPath, [System.Drawing.Imaging.ImageFormat]::Png)
        if ((Get-Item $outputPath).Length -lt 20000) { throw "Capture failed: $outputPath" }
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

Invoke-CommandId 177 # Reading with comments.
Invoke-CommandId 187 # WYSIWYG.
Invoke-CommandId 172 # Outline.
Save-Frame
Write-Output $outputPath
} finally {
    if ($null -eq $originalTheme) { Remove-ItemProperty $settingsKey Theme -ErrorAction SilentlyContinue }
    else { Set-ItemProperty $settingsKey Theme $originalTheme }
    if ($null -eq $originalAppearance) { Remove-ItemProperty $settingsKey EditorAppearance -ErrorAction SilentlyContinue }
    else { Set-ItemProperty $settingsKey EditorAppearance $originalAppearance }
}
