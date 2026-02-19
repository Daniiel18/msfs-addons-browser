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
                string currentVersion =
                    typeof(App).Assembly.GetName().Version?.ToString() ?? "Unknown";

                string newVersion =
                    update.TargetFullRelease.Version.ToString();

                if (settings.IgnoredUpdateVersion != newVersion)
                {
                    splash.Close();

                    var changelog = new List<string>
            {
                update.TargetFullRelease.NotesHTML ?? "No changelog provided."
            };

                    var dialog = new Views.UpdateDialog(
                        currentVersion,
                        newVersion,
                        changelog
                    );

                    bool? result = dialog.ShowDialog();

                    if (result == true && dialog.ShouldUpdate)
                    {
                        await updateService.ApplyUpdateAsync(update);
                        return;
                    }

                    settings.IgnoredUpdateVersion = newVersion;
                    settingsService.Save(settings);
                }
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
