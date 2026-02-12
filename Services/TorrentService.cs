using MonoTorrent;
using MonoTorrent.Client;
using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using System.IO;

namespace SceneryAddonsBrowser.Services
{
    public class TorrentService
    {
        private ClientEngine? _engine;
        private TorrentManager? _manager;
        private CancellationTokenSource? _cts;

        public bool IsDownloading => _manager != null;
        public bool IsPaused => _manager?.State == TorrentState.Paused;

        private void EnsureEngine(string downloadPath)
        {
            if (_engine != null)
                return;

            AppLogger.Log("[TORRENT] Initializing new engine");

            var settings = new EngineSettingsBuilder
            {
                CacheDirectory = Path.Combine(downloadPath, ".cache")
            }.ToSettings();

            _engine = new ClientEngine(settings);
        }

        public async Task DownloadFromMagnetAsync(
            string magnetUri,
            string downloadPath,
            Action<DownloadProgress>? progressCallback,
            CancellationToken externalToken)
        {
            Directory.CreateDirectory(downloadPath);

            _cts = CancellationTokenSource.CreateLinkedTokenSource(externalToken);
            var token = _cts.Token;

            EnsureEngine(downloadPath);

            // 🔒 Si hay un manager previo, lo detenemos y lo quitamos del engine
            if (_manager != null)
            {
                AppLogger.Log("[TORRENT] Existing manager detected. Stopping...");
                await StopInternalAsync();
            }

            var magnet = MagnetLink.Parse(magnetUri);

            _manager = await _engine!.AddAsync(magnet, downloadPath);

            AppLogger.Log("[TORRENT] Torrent manager created. Starting download...");
            await _manager.StartAsync();

            try
            {
                long lastBytes = 0;
                var lastTime = DateTime.UtcNow;

                while (!_manager.Complete && !token.IsCancellationRequested)
                {
                    await Task.Delay(500, token);

                    var now = DateTime.UtcNow;
                    var bytes = _manager.Monitor.DataBytesDownloaded;

                    var deltaBytes = bytes - lastBytes;
                    var deltaSeconds = (now - lastTime).TotalSeconds;

                    double speedMbps = deltaSeconds > 0
                        ? (deltaBytes * 8d) / (1024 * 1024) / deltaSeconds
                        : 0;

                    lastBytes = bytes;
                    lastTime = now;

                    TimeSpan eta = TimeSpan.Zero;
                    if (speedMbps > 0 && _manager.Torrent != null)
                    {
                        var remaining = _manager.Torrent.Size - bytes;
                        var bytesPerSec = (speedMbps * 1024 * 1024) / 8;
                        eta = TimeSpan.FromSeconds(remaining / bytesPerSec);
                    }

                    progressCallback?.Invoke(new DownloadProgress
                    {
                        Percent = _manager.Progress,
                        SpeedMbps = speedMbps,
                        Eta = eta,
                        Status = _manager.State == TorrentState.Paused
                            ? "Paused"
                            : "Downloading"
                    });
                }
            }
            finally
            {
                await StopInternalAsync();
            }
        }


        public async Task PauseAsync()
        {
            if (_manager?.State == TorrentState.Downloading)
                await _manager.PauseAsync();
        }

        public async Task ResumeAsync()
        {
            if (_manager?.State == TorrentState.Paused)
                await _manager.StartAsync();
        }

        public async Task CancelAsync()
        {
            AppLogger.Log("[TORRENT] Cancel requested");
            _cts?.Cancel();
        }

        private async Task StopInternalAsync()
        {
            if (_manager != null)
            {
                try
                {
                    await _manager.StopAsync();

                    if (_engine != null)
                    {
                        await _engine.RemoveAsync(_manager);
                    }
                }
                catch (Exception ex)
                {
                    AppLogger.LogError("[TORRENT] Failed to stop/remove manager", ex);
                }
            }

            _manager = null;
            _cts = null;
        }
    }
}
