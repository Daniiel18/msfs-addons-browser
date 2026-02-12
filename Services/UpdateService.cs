using Velopack;
using Velopack.Sources;

namespace SceneryAddonsBrowser.Services
{
    public class UpdateService
    {
        public async Task<bool> CheckAndApplyUpdatesAsync()
        {
            try
            {
                var source = new GithubSource(
                    "https://github.com/Daniiel18/SceneryAddonsBrowser",
                    accessToken: null,
                    prerelease: false
                );

                var manager = new UpdateManager(source);

                var update = await manager.CheckForUpdatesAsync();
                if (update == null)
                    return false; 

                await manager.DownloadUpdatesAsync(update);

               
                manager.ApplyUpdatesAndRestart(update);

                return true;
            }
            catch
            {
                return false;
            }
        }
    }
}
