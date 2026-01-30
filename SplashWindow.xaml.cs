using System.Threading.Tasks;
using System.Windows;

namespace SceneryAddonsBrowser
{
    public partial class SplashWindow : Window
    {
        public SplashWindow()
        {
            InitializeComponent();
            Loaded += SplashWindow_Loaded;
        }

        private async void SplashWindow_Loaded(object sender, RoutedEventArgs e)
        {
            await Task.Delay(3000); // Simulate some loading time

            var mainWindow = new MainWindow();
            mainWindow.Show();

            Close();
        }
    }
}
