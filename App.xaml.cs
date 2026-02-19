using SceneryAddonsBrowser.Services;
using System.Windows;
using Application = System.Windows.Application;

namespace SceneryAddonsBrowser
{
    public partial class App : Application
    {
        protected override async void OnStartup(StartupEventArgs e)
        {
            base.OnStartup(e);

            var splash = new SplashWindow();
            splash.Show();
            await splash.RunAsync();

            var settingsService = new SettingsService();
            var settings = settingsService.Load();

            var updateService = new UpdateService();
            var update = await updateService.CheckForUpdatesAsync();

            if (update != null)
            {
                PendingUpdateStore.PendingUpdate = update;
            }

            if (!string.IsNullOrWhiteSpace(settings.DownloadRoot))
            {
                UserStorage.SetRoot(settings.DownloadRoot);
            }

            var mainWindow = new MainWindow();
            MainWindow = mainWindow;
            mainWindow.Show();

            splash.Close();
        }

        protected override void OnExit(ExitEventArgs e)
        {
            base.OnExit(e);
            Environment.Exit(0);
        }
    }
}
