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

                    var changelog = ChangelogParser.Parse(
                        update.TargetFullRelease.NotesHTML
                    );

                    var dialog = new UpdateDialog(
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

            splash.Close();

            if (string.IsNullOrWhiteSpace(settings.DownloadRoot))
            {
                var dialog = new System.Windows.Forms.FolderBrowserDialog
                {
                    Description = "Select storage folder for Scenery Addons Browser"
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
        }

        protected override void OnExit(ExitEventArgs e)
        {
            base.OnExit(e);
            Environment.Exit(0);
        }
    }
}
