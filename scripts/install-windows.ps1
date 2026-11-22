param(
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Programs\pane-md'),
    [switch]$RegisterOnly
)

$ErrorActionPreference = 'Stop'
$source = Split-Path -Parent $PSCommandPath
if (-not $RegisterOnly) {
    New-Item -ItemType Directory -Force -Path $InstallDirectory | Out-Null
    if ([IO.Path]::GetFullPath($source) -ne [IO.Path]::GetFullPath($InstallDirectory)) {
        foreach ($name in @('pane-md.exe', 'pane-md-editor.exe')) {
            Copy-Item -LiteralPath (Join-Path $source $name) -Destination (Join-Path $InstallDirectory $name) -Force
        }
    }
}

$main = Join-Path $InstallDirectory 'pane-md.exe'
$launcher = Join-Path $InstallDirectory 'pane-md-editor.exe'
if (-not (Test-Path -LiteralPath $main -PathType Leaf) -or
    -not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw 'pane-md.exe and pane-md-editor.exe must both exist in the install directory.'
}

function Set-RegistryValue([string]$key, [string]$name, [object]$value,
                            [Microsoft.Win32.RegistryValueKind]$kind) {
    $handle = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($key)
    try { $handle.SetValue($name, $value, $kind) }
    finally { $handle.Dispose() }
}

$progid = 'pane-md.Markdown'
$command = '"' + $launcher + '" --open "%1"'
$icon = '"' + $main + '",0'
$extensions = @('.md', '.markdown', '.mdown', '.mkd')

Set-RegistryValue 'Software\Classes\pane-md.Markdown' '' 'Markdown document' String
Set-RegistryValue 'Software\Classes\pane-md.Markdown\DefaultIcon' '' $icon String
Set-RegistryValue 'Software\Classes\pane-md.Markdown\shell\open\command' '' $command String

Set-RegistryValue 'Software\Classes\Applications\pane-md-editor.exe' 'FriendlyAppName' 'pane-md' String
Set-RegistryValue 'Software\Classes\Applications\pane-md-editor.exe\DefaultIcon' '' $icon String
Set-RegistryValue 'Software\Classes\Applications\pane-md-editor.exe\shell\open\command' '' $command String
foreach ($extension in $extensions) {
    Set-RegistryValue "Software\Classes\Applications\pane-md-editor.exe\SupportedTypes" $extension '' String
    Set-RegistryValue "Software\Classes\$extension\OpenWithProgids" $progid ([byte[]]@()) None
}

Set-RegistryValue 'Software\pane-md\Capabilities' 'ApplicationName' 'pane-md' String
Set-RegistryValue 'Software\pane-md\Capabilities' 'ApplicationDescription' 'Markdown editor with source, WYSIWYG, and in-file comments' String
Set-RegistryValue 'Software\pane-md\Capabilities' 'ApplicationIcon' $icon String
foreach ($extension in $extensions) {
    Set-RegistryValue 'Software\pane-md\Capabilities\FileAssociations' $extension $progid String
}
Set-RegistryValue 'Software\RegisteredApplications' 'pane-md' 'Software\pane-md\Capabilities' String
Set-RegistryValue 'Software\Microsoft\Windows\CurrentVersion\App Paths\pane-md.exe' '' $main String
Set-RegistryValue 'Software\Microsoft\Windows\CurrentVersion\App Paths\pane-md-editor.exe' '' $launcher String

$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\pane-md.lnk'
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($startMenu)
$shortcut.TargetPath = $launcher
$shortcut.Arguments = '--open'
$shortcut.IconLocation = "$main,0"
$shortcut.WorkingDirectory = $InstallDirectory
$shortcut.Save()

if (-not ('PaneMd.Shell' -as [type])) {
    Add-Type -Namespace PaneMd -Name Shell -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll")]
public static extern void SHChangeNotify(uint eventId, uint flags, System.IntPtr item1, System.IntPtr item2);
'@
}
[PaneMd.Shell]::SHChangeNotify(0x08000000, 0x1003, [IntPtr]::Zero, [IntPtr]::Zero)
Write-Output "pane-md registered for $($extensions -join ', '). Choose it in Windows Default apps to make it the default."
