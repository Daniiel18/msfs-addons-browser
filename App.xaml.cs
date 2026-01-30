using System.Windows;

namespace SceneryAddonsBrowser
{
    public partial class App : Application
    {
        private void Application_Startup(object sender, StartupEventArgs e)
        {
            var splash = new SplashWindow();
            splash.Show();
        }
    }
}
