[CmdletBinding()]
param([ValidateRange(1,120)][int]$Seconds = 45)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Standalone test stimulus. This creates/closes only its own non-activated
# window and performs no input automation or desktop/user-data capture.
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    throw 'moving_source_requires_sta'
}
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$taskHelper = Join-Path $PSScriptRoot 'moving-source-fast.cs'
Add-Type -Path $taskHelper -ReferencedAssemblies @('System.dll','System.Windows.Forms.dll','System.Drawing.dll')
$taskReport = [Mrd.Testing.NativeMovingSource]::Run($Seconds)
[ordered]@{
    event = 'native_moving_source_finished'
    source_backend = 'compiled_csharp_winforms_gdi'
    requested_seconds = $Seconds
    window_width = 900
    window_height = 320
    window_left = 20
    window_top = 20
    topmost = $true
    show_activated = $false
    update_callbacks = $taskReport.UpdateCallbacks
    painted_updates = $taskReport.PaintedUpdates
    elapsed_ms = $taskReport.ElapsedMilliseconds
    first_paint_elapsed_ms = $taskReport.FirstPaintMilliseconds
    last_paint_elapsed_ms = $taskReport.LastPaintMilliseconds
    painted_update_hz = $taskReport.PaintedUpdateHz
    target_content_update_hz = 60
    physical_display_presentations_measured = $false
    input_automation = $false
    desktop_pixels_read = $false
} | ConvertTo-Json -Depth 4
if ($taskReport.PaintedUpdateHz -lt 50) { exit 1 }
