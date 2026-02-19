using SceneryAddonsBrowser.Models;
using SceneryAddonsBrowser.Services;
using SceneryAddonsBrowser.Update;
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
                    var changelog = ChangelogParser.Parse(
                        update.TargetFullRelease.NotesHTML
                    );

                    var pending = new PendingUpdate(
                        update,
                        currentVersion,
                        changelog
                    );

                    PendingUpdateStore.PendingUpdate = pending;

                    var dialog = new Views.UpdateDialog(
                        currentVersion,
                        newVersion,
                        changelog
                    );

                    dialog.Owner = splash;

                    bool? result = dialog.ShowDialog();

                    if (result == true && dialog.ShouldUpdate)
                    {
                        await updateService.ApplyUpdateAsync(update);
                        return; // Velopack reinicia
                    }

                    settings.IgnoredUpdateVersion = newVersion;
                    settingsService.Save(settings);
                }
            }

            splash.Close(); // ⬅️ SOLO AQUÍ

            UserStorage.SetRoot(settings.DownloadRoot);

            var mainWindow = new MainWindow();
            MainWindow = mainWindow;
            mainWindow.Show();
        }

        protected override void OnExit(ExitEventArgs e)
        {
            base.OnExit(e);
            Environment.Exit(0);
        }
    }
}
