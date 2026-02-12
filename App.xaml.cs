using SceneryAddonsBrowser.Services;
using System.Threading.Tasks;
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

            try
            {
                var updater = new UpdateService();
                bool updating = await updater.CheckAndApplyUpdatesAsync();

                if (updating)
                    return; 
            }
            catch
            {
            }

            var settingsService = new SettingsService();
            var settings = settingsService.Load();

            if (string.IsNullOrWhiteSpace(settings.DownloadRoot))
            {
                var dialog = new System.Windows.Forms.FolderBrowserDialog
                {
                    Description = "Select storage folder for SceneryAddonsBrowser"
                };

                if (dialog.ShowDialog() != System.Windows.Forms.DialogResult.OK)
                {
                    Shutdown();
                    return;
                }

                settings.DownloadRoot = dialog.SelectedPath;
                settingsService.Save(settings);
            }

            UserStorage.SetRoot(settings.DownloadRoot);

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
