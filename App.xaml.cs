using SceneryAddonsBrowser.Models;
using SceneryAddonsBrowser.Services;
using SceneryAddonsBrowser.Update;
using SceneryAddonsBrowser.Views;
using System.Reflection;
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
                    Assembly
                        .GetExecutingAssembly()
                        .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?
                        .InformationalVersion
                    ?? "Unknown";

                string newVersion = update.TargetFullRelease.Version.ToString();

                if (settings.IgnoredUpdateVersion != newVersion)
                {
                    splash.Close();

                    var changelog = new List<string>
{
    "Test line 1",
    "Test line 2",
    "Test line 3"
};

                    var dialog = new UpdateDialog(
                        currentVersion,
                        newVersion,
                        changelog
                    );

                    dialog.Owner = splash;

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
