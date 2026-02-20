using SceneryAddonsBrowser.Models;
using SceneryAddonsBrowser.Services;
using SceneryAddonsBrowser.Update;
using SceneryAddonsBrowser.Views;
using System.Reflection;
using System.Windows;
using Application = System.Windows.Application;
using SceneryAddonsBrowser.Logging;

namespace SceneryAddonsBrowser
{
    public partial class App : Application
    {
        protected override async void OnStartup(StartupEventArgs e)
        {
            base.OnStartup(e);

            AppLogger.Log("App: Startup");

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
                    ?? Assembly.GetExecutingAssembly().GetName().Version?.ToString()
                    ?? "Unknown";

                currentVersion = NormalizeVersion(currentVersion);

                string newVersion = NormalizeVersion(
                    update.TargetFullRelease.Version.ToString()
                );

                if (settings.IgnoredUpdateVersion != newVersion)
                {
                    splash.Close(); 

                    AppLogger.Log($"App: Update available. Current={currentVersion}, New={newVersion}");

                    var notesHtml = Update.ReleaseNotesProvider.Load(newVersion);
                    var changelog = Update.ChangelogParser.Parse(notesHtml);

                    PendingUpdateStore.PendingUpdate = new PendingUpdate(
                        update,
                        currentVersion,
                        changelog
                    );

                    var dialog = new UpdateDialog(
                        currentVersion,
                        newVersion,
                        changelog
                    )
                    {
                        WindowStartupLocation = WindowStartupLocation.CenterScreen
                    };

                    bool? result = dialog.ShowDialog();

                    AppLogger.Log($"App: Update dialog closed. Result={(result == true ? "Accept" : "Decline")}");

                    if (result == true && dialog.ShouldUpdate)
                    {
                        AppLogger.Log("App: User accepted update - applying");
                        await updateService.ApplyUpdateAsync(update);
                        return; 
                    }
                }
            }

            splash.Close();

            if (string.IsNullOrWhiteSpace(settings.DownloadRoot))
            {
                AppLogger.Log("App: Prompting user to select download root");
                var dialog = new System.Windows.Forms.FolderBrowserDialog
                {
                    Description = "Select storage folder for Scenery Addons Browser"
                };

                if (dialog.ShowDialog() != System.Windows.Forms.DialogResult.OK)
                {
                    AppLogger.Log("App: User declined to select storage folder - shutting down");
                    Shutdown();
                    return;
                }

                settings.DownloadRoot = dialog.SelectedPath;
                settingsService.Save(settings);

                AppLogger.Log($"App: User selected storage folder: {dialog.SelectedPath}");
            }

            UserStorage.SetRoot(settings.DownloadRoot);

            var mainWindow = new MainWindow();
            MainWindow = mainWindow;
            mainWindow.Show();

            AppLogger.Log("App: Main window shown");
        }

        private static string NormalizeVersion(string version)
        {
            var plusIndex = version.IndexOf('+');
            return plusIndex > 0 ? version[..plusIndex] : version;
        }


        protected override void OnExit(ExitEventArgs e)
        {
            base.OnExit(e);
            AppLogger.Log("App: Exit");
            Environment.Exit(0);
        }
    }
}
