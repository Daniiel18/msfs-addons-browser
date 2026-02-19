using SceneryAddonsBrowser.Logging;
using Velopack;
using Velopack.Sources;

namespace SceneryAddonsBrowser.Services
{
    public class UpdateService
    {
        private readonly UpdateManager _manager;

        public UpdateService()
        {
            var source = new GithubSource(
                "https://github.com/Daniiel18/SceneryAddonsBrowser",
                accessToken: null,
                prerelease: false
            );

            _manager = new UpdateManager(source);
        }

        public async Task<UpdateInfo?> CheckForUpdatesAsync()
        {
            try
            {
                AppLogger.Log("[UPDATE] Checking for updates...");
                return await _manager.CheckForUpdatesAsync();
            }
            catch (Exception ex)
            {
                AppLogger.LogError("[UPDATE] Update check failed", ex);
                return null;
            }
        }

        public async Task ApplyUpdateAsync(UpdateInfo update)
        {
            AppLogger.Log("[UPDATE] Downloading update...");
            await _manager.DownloadUpdatesAsync(update);

            AppLogger.Log("[UPDATE] Applying update and restarting...");
            _manager.ApplyUpdatesAndRestart(update);
        }
    }
}
