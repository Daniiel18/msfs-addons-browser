using System.Threading.Tasks;
using System.Windows;

namespace SceneryAddonsBrowser
{
    public partial class SplashWindow : Window
    {
        public SplashWindow()
        {
            InitializeComponent();
        }

        public async Task RunAsync()
        {
            await Step("Initializing...", 15, 800);
            await Step("Loading services...", 35, 900);
            await Step("Checking updates...", 60, 1000);
            await Step("Preparing UI...", 85, 900);
            await Step("Starting application...", 100, 600);
        }

        private async Task Step(string text, int progress, int delayMs)
        {
            StatusText.Text = text;
            ProgressBar.Value = progress;
            await Task.Delay(delayMs);
        }
    }
}
