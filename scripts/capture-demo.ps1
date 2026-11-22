param(
    [string]$Document = 'examples/review-demo.md',
    [string]$Output = 'docs/media'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$documentPath = (Resolve-Path (Join-Path $root $Document)).Path
$outputPath = Join-Path $root $Output
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null

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
}
'@
[DemoWindow]::SetProcessDPIAware() | Out-Null

Get-Process pane-md -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowTitle -like 'review-demo.md * pane-md' } |
    ForEach-Object { $_.CloseMainWindow() | Out-Null; $_.WaitForExit(5000) | Out-Null }

Start-Process -FilePath (Join-Path $root 'target/release/pane-md.exe') -ArgumentList ('"' + $documentPath + '"') | Out-Null
$window = $null
for ($attempt = 0; $attempt -lt 40; $attempt++) {
    $window = Get-Process pane-md -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowTitle -like 'review-demo.md * pane-md' } |
        Select-Object -First 1
    if ($window) { break }
    Start-Sleep -Milliseconds 250
}
if (-not $window) { throw 'Demo window did not open' }
$handle = $window.MainWindowHandle
[DemoWindow]::ShowWindow($handle, 9) | Out-Null
[DemoWindow]::SetWindowPos($handle, [IntPtr]::Zero, 90, 65, 1440, 900, 0x40) | Out-Null
[DemoWindow]::SetForegroundWindow($handle) | Out-Null

function Invoke-CommandId([int]$id) {
    [DemoWindow]::SendMessage($handle, 0x111, [UIntPtr]::new([uint32]$id), [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 650
}

function Save-Frame([string]$name) {
    [DemoWindow]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 350
    $rect = New-Object DemoWindow+Rect
    [DemoWindow]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $outputPath $name), [System.Drawing.Imaging.ImageFormat]::Png)
        if ((Get-Item (Join-Path $outputPath $name)).Length -lt 20000) { throw "Capture failed: $name" }
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

Invoke-CommandId 177 # Reading with comments.
Invoke-CommandId 187 # WYSIWYG.
Invoke-CommandId 172 # Outline.
Save-Frame '01-wysiwyg.png'
Invoke-CommandId 172 # Hide outline.
Invoke-CommandId 188 # Markdown source.
Save-Frame '02-source.png'
Invoke-CommandId 178 # Editing with comments.
Save-Frame '03-editing.png'
$window.CloseMainWindow() | Out-Null
