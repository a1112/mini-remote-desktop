param([int]$Seconds = 45)
$ErrorActionPreference = 'Stop'
# A temporary test application, not an input automation tool. Its own window
# creates real desktop updates on the primary monitor and closes automatically.
Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase
$sourceWindow = New-Object Windows.Window
$sourceWindow.Title = 'Rdesk capture test — closes automatically'
$sourceWindow.Width = 900
$sourceWindow.Height = 320
$sourceWindow.Left = 20
$sourceWindow.Top = 20
$sourceWindow.Topmost = $true
$sourceWindow.ShowActivated = $false
$sourceWindow.ResizeMode = 'NoResize'
$sourceLabel = New-Object Windows.Controls.TextBlock
$sourceLabel.FontSize = 32
$sourceLabel.Foreground = [Windows.Media.Brushes]::White
$sourceLabel.Padding = '20'
$sourceWindow.Content = $sourceLabel
$sourceClock = [Diagnostics.Stopwatch]::StartNew()
$sourceFrame = 0
$sourceHandler = [EventHandler]{
    if ($sourceClock.Elapsed.TotalSeconds -ge $Seconds) { $sourceWindow.Close(); return }
    $script:sourceFrame++
    $color = [Windows.Media.Color]::FromRgb(
        [byte](32 + ($script:sourceFrame * 7) % 180),
        [byte](32 + ($script:sourceFrame * 11) % 180),
        [byte](32 + ($script:sourceFrame * 17) % 180))
    $sourceWindow.Background = New-Object Windows.Media.SolidColorBrush $color
    $sourceLabel.Text = "Rdesk real desktop capture test`nFrame $script:sourceFrame`nCloses after $Seconds seconds"
}
[Windows.Media.CompositionTarget]::add_Rendering($sourceHandler)
try { $sourceWindow.ShowDialog() | Out-Null }
finally { [Windows.Media.CompositionTarget]::remove_Rendering($sourceHandler) }
Write-Output "Source rendered $sourceFrame updates in $($sourceClock.Elapsed.TotalSeconds) seconds"
