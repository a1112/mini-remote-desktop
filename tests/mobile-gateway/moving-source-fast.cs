using System;
using System.Diagnostics;
using System.Drawing;
using System.Windows.Forms;

namespace Mrd.Testing
{
    public sealed class MovingSourceReport
    {
        public long UpdateCallbacks;
        public long PaintedUpdates;
        public double ElapsedMilliseconds;
        public double FirstPaintMilliseconds;
        public double LastPaintMilliseconds;
        public double PaintedUpdateHz;
    }

    internal sealed class MovingSourceForm : Form
    {
        internal readonly Stopwatch Clock = new Stopwatch();
        internal long State;
        internal long PaintedUpdates;
        internal double FirstPaintMilliseconds;
        internal double LastPaintMilliseconds;
        private long lastPaintedState = -1;
        private readonly SolidBrush[] backgrounds = {
            new SolidBrush(Color.FromArgb(20, 35, 62)),
            new SolidBrush(Color.FromArgb(28, 49, 76)),
            new SolidBrush(Color.FromArgb(34, 57, 84)),
            new SolidBrush(Color.FromArgb(24, 42, 70))
        };
        private readonly SolidBrush orange = new SolidBrush(Color.FromArgb(255, 162, 42));
        private readonly SolidBrush cyan = new SolidBrush(Color.FromArgb(30, 218, 230));
        private readonly SolidBrush zero = new SolidBrush(Color.FromArgb(42, 55, 73));
        private readonly SolidBrush one = new SolidBrush(Color.FromArgb(248, 250, 255));
        private readonly Font heading = new Font("Segoe UI", 18);

        internal MovingSourceForm()
        {
            Text = "Rdesk native motion test - closes automatically";
            ClientSize = new Size(900, 320);
            Location = new Point(20, 20);
            StartPosition = FormStartPosition.Manual;
            FormBorderStyle = FormBorderStyle.None;
            TopMost = true;
            ShowInTaskbar = false;
            DoubleBuffered = true;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint, true);
        }
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override CreateParams CreateParams
        {
            get {
                CreateParams parameters = base.CreateParams;
                parameters.ExStyle |= 0x08000000; // WS_EX_NOACTIVATE, this owned window only.
                return parameters;
            }
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics graphics = e.Graphics;
            graphics.FillRectangle(backgrounds[State % backgrounds.Length], ClientRectangle);
            graphics.DrawString("Rdesk native moving source - closes automatically", heading, one, 20, 18);
            int span = Math.Max(1, ClientSize.Width - 110);
            int x = (int)((State * 13) % span);
            graphics.FillRectangle(orange, x + 10, 85, 90, 70);
            graphics.FillRectangle(cyan, span - x + 10, 178, 90, 54);
            // Each content state has a unique in-picture counter. No desktop
            // pixels are read, copied or persisted by this test application.
            for (int bit = 0; bit < 16; bit++)
                graphics.FillRectangle((State & (1L << bit)) == 0 ? zero : one,
                    20 + bit * 38, 268, 30, 25);
            if (Clock.IsRunning && State != lastPaintedState)
            {
                double elapsed = Clock.Elapsed.TotalMilliseconds;
                if (PaintedUpdates == 0) FirstPaintMilliseconds = elapsed;
                LastPaintMilliseconds = elapsed;
                PaintedUpdates++;
                lastPaintedState = State;
            }
        }
        protected override void Dispose(bool disposing)
        {
            if (disposing) {
                foreach (SolidBrush brush in backgrounds) brush.Dispose();
                orange.Dispose(); cyan.Dispose(); zero.Dispose(); one.Dispose(); heading.Dispose();
            }
            base.Dispose(disposing);
        }
    }

    public static class NativeMovingSource
    {
        public static MovingSourceReport Run(int seconds)
        {
            if (seconds < 1 || seconds > 120) throw new ArgumentOutOfRangeException("seconds");
            Application.EnableVisualStyles();
            long callbacks = 0;
            using (MovingSourceForm form = new MovingSourceForm())
            using (Timer timer = new Timer())
            {
                timer.Interval = 1;
                timer.Tick += delegate {
                    callbacks++;
                    if (form.Clock.Elapsed.TotalSeconds >= seconds) { form.Close(); return; }
                    long state = (long)(form.Clock.Elapsed.TotalSeconds * 60);
                    if (state <= form.State) return;
                    form.State = state;
                    // Refresh synchronously paints this window, rather than
                    // relying on the throttled WPF composition event clock.
                    form.Refresh();
                };
                form.Shown += delegate { form.Clock.Start(); form.Refresh(); timer.Start(); };
                try { Application.Run(form); }
                finally { timer.Stop(); form.Clock.Stop(); }
                double span = form.LastPaintMilliseconds - form.FirstPaintMilliseconds;
                return new MovingSourceReport {
                    UpdateCallbacks = callbacks,
                    PaintedUpdates = form.PaintedUpdates,
                    ElapsedMilliseconds = form.Clock.Elapsed.TotalMilliseconds,
                    FirstPaintMilliseconds = form.FirstPaintMilliseconds,
                    LastPaintMilliseconds = form.LastPaintMilliseconds,
                    PaintedUpdateHz = form.PaintedUpdates > 1 && span > 0
                        ? (form.PaintedUpdates - 1) * 1000.0 / span : 0
                };
            }
        }
    }
}
