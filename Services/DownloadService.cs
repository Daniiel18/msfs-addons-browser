using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using System.Diagnostics;
using System.IO;
using System.Net.Http;
using System.Windows;
using MessageBox = System.Windows.MessageBox;

namespace SceneryAddonsBrowser.Services
{
    public class DownloadService
    {
        private readonly HttpClient _httpClient;
        private readonly HistoryService _historyService = new();
        private readonly TorrentService _torrentService = new();
        private readonly TorrentQueueService _torrentQueue;
        private readonly DownloadSessionService _sessionService = new();

        private string? _currentDataDir;

        public int QueueTotal => _torrentQueue.QueueCount + (IsDownloading ? 1 : 0);
        public int QueueCurrent => IsDownloading ? 1 : 0;

        public DownloadState CurrentState { get; private set; } = DownloadState.Queued;
        public bool IsDownloading => CurrentState == DownloadState.Downloading;
        public int QueueCount => _torrentQueue.QueueCount;

        private string? _lastMagnet;


        public event Action<DownloadState>? StateChanged;

        public event Action? QueueChanged;

        public DownloadService()
        {
            _httpClient = new HttpClient(
                new HttpClientHandler
                {
                    AllowAutoRedirect = false
                });

            _torrentQueue = new TorrentQueueService(this);
            _torrentQueue.QueueChanged += () => QueueChanged?.Invoke();

        }

        private volatile bool _isAppClosing = false;

        public void NotifyAppClosing()
        {
            _isAppClosing = true;
        }

        private void SetState(DownloadState state)
        {
            CurrentState = state;
            AppLogger.Log($"Download state changed to: {state}");
            StateChanged?.Invoke(state);
        }

        // ================= CONTROLS =================
        public Task PauseCurrentDownloadAsync()
        {
            AppLogger.Log("Pausing current download...");
            SetState(DownloadState.Paused);
            return _torrentService.PauseAsync();
        }

        public Task ResumeCurrentDownloadAsync()
        {
            AppLogger.Log("Resuming current download...");
            SetState(DownloadState.Downloading);
            return _torrentService.ResumeAsync();
        }

        public async Task CancelCurrentDownloadAsync()
        {
            AppLogger.Log("Cancelling current download...");

            await _torrentService.CancelAsync();

            var session = _sessionService.Load();
            if (session != null)
            {
                try
                {
                    if (Directory.Exists(session.DataPath))
                    {
                        AppLogger.Log($"Deleting download directory: {session.DataPath}");
                        Directory.Delete(session.DataPath, true);
                    }
                }
                catch (Exception ex)
                {
                    AppLogger.LogError("Failed to delete download directory", ex);
                }

                _sessionService.Clear();
            }

            SetState(DownloadState.Cancelled);
            CleanupSceneriesRoot();
        }

        public void SaveCurrentSessionIfNeeded()
        {
            if (CurrentState != DownloadState.Downloading &&
                CurrentState != DownloadState.Paused)
                return;

            var session = _sessionService.Load();
            if (session != null)
                return; // ya existe, no duplicar

            if (string.IsNullOrWhiteSpace(_currentDataDir))
                return;

            AppLogger.Log("[SESSION] Saving active download session");

            _sessionService.Save(new DownloadSession
            {
                ScenarioId = Path.GetFileName(
                    Path.GetDirectoryName(_currentDataDir)!),
                MagnetUri = _lastMagnet!,   // ver paso 2
                DataPath = _currentDataDir,
                IsCompleted = false
            });
        }

        // ================= TORRENT =================
        internal async Task DownloadTorrentInternalAsync(DownloadMethod method,Action<string, int>? progress)
        {
            if (method.Scenario == null)
                throw new InvalidOperationException("Scenario missing.");

            var scenarioId = $"{method.Scenario.Icao}_{method.Scenario.Developer}"
                .Replace(" ", "_");

            var dataDir = AppPaths.GetScenarioDataDir(scenarioId);

            AppLogger.Log($"[TORRENT] ScenarioId = {scenarioId}");
            AppLogger.Log($"[TORRENT] DataDir = {dataDir}");

            Directory.CreateDirectory(dataDir);

            try
            {
                SetState(DownloadState.ResolvingMagnet);
                progress?.Invoke("Resolving magnet...", 0);

                var magnet = await ResolveMagnetAsync(method.Url);

                _lastMagnet = magnet;
                _currentDataDir = dataDir;

                _sessionService.Save(new DownloadSession
                {
                    ScenarioId = scenarioId,
                    MagnetUri = magnet,
                    DataPath = dataDir,
                    IsCompleted = false
                });

                SetState(DownloadState.Downloading);

                await _torrentService.DownloadFromMagnetAsync(
                    magnet,
                    dataDir,
                    p =>
                    {
                        progress?.Invoke(
                            $"Downloading • {p.Percent:F1}% • {p.SpeedMbps:F1} Mbps • ETA {p.Eta:mm\\:ss}",
                            (int)p.Percent
                        );
                    },
                    CancellationToken.None
                );

                if (CurrentState == DownloadState.Cancelled)
                {
                    AppLogger.Log("[TORRENT] Download cancelled. Aborting pipeline.");
                    return;
                }

                AppLogger.Log("[TORRENT] Torrent download completed.");

                await ContinueInstallationAsync(new DownloadSession
                {
                    ScenarioId = scenarioId,
                    MagnetUri = magnet,
                    DataPath = dataDir,
                    IsCompleted = false
                });
            }
            catch (Exception ex)
            {
                AppLogger.LogError("[TORRENT] Torrent download failed", ex);
                SetState(DownloadState.Error);
            }
        }

        private void CleanupSceneriesRoot()
        {
            try
            {
                var root = AppPaths.GetSceneriesRoot();

                if (Directory.Exists(root))
                {
                    Directory.Delete(root, true);
                    AppLogger.Log($"Sceneries root cleaned: {root}");
                }
            }
            catch (Exception ex)
            {
                AppLogger.LogError("Failed to clean sceneries root", ex);
            }
        }

        // ================= ENTRY POINT =================
        public async Task StartDownloadAsync(DownloadMethod method, Action<string, int>? progressCallback = null)
        {
            AppLogger.Log("=== StartDownloadAsync ===");

            if (method?.Scenario == null)
                return;

            if (method.Type == DownloadType.Torrent)
            {
                _torrentQueue.Enqueue(new TorrentJob(method, progressCallback));
                QueueChanged?.Invoke();
                return;
            }

            progressCallback?.Invoke($"Starting {method.Name}...", 0);

            if (method.Type == DownloadType.Torrent)
            {
                _torrentQueue.Enqueue(new TorrentJob(method, progressCallback));
                return;
            }

            // Mirror
            Process.Start(new ProcessStartInfo
            {
                FileName = method.Url,
                UseShellExecute = true
            });
        }

        // ================= MAGNET =================
        private async Task<string> ResolveMagnetAsync(string getUrl)
        {
            AppLogger.Log($"Resolving magnet from: {getUrl}");

            var html = await _httpClient.GetStringAsync(getUrl);

            int magnetIndex = html.IndexOf("magnet:?", StringComparison.OrdinalIgnoreCase);
            if (magnetIndex < 0)
                throw new Exception("Magnet link not found in HTML.");

            var magnet = html.Substring(magnetIndex);
            int endIndex = magnet.IndexOf('"');
            if (endIndex < 0) endIndex = magnet.IndexOf('\'');

            if (endIndex > 0)
                magnet = magnet.Substring(0, endIndex);

            AppLogger.Log($"Resolved magnet: {magnet}");
            return magnet.Trim();
        }

        public async Task ResumeTorrentSessionAsync(Action<string, int>? progress = null)
        {
            var session = _sessionService.Load();
            if (session == null)
                return;

            AppLogger.Log("=== RESUME TORRENT SESSION ===");
            AppLogger.Log($"Magnet: {session.MagnetUri}");
            AppLogger.Log($"DataPath: {session.DataPath}");

            SetState(DownloadState.Downloading);

            await _torrentService.DownloadFromMagnetAsync(
                session.MagnetUri,
                session.DataPath,
                p =>
                {
                    int percent = (int)Math.Round(p.Percent);

                    // 🔴 ESTO ES LO QUE FALTABA
                    progress?.Invoke(
                        $"Resuming • {p.Percent:F1}% • {p.SpeedMbps:F1} Mbps • ETA {p.Eta:mm\\:ss}",
                        percent
                    );
                },
                CancellationToken.None
            );

            _sessionService.Clear();
        }

        private async Task ContinueInstallationAsync(DownloadSession session)
        {
            AppLogger.Log("[INSTALL] ContinueInstallationAsync");

            var finalPackage = Directory
                .EnumerateFiles(session.DataPath, "*.*", SearchOption.AllDirectories)
                .FirstOrDefault(f =>
                    f.EndsWith(".zip", StringComparison.OrdinalIgnoreCase) ||
                    f.EndsWith(".rar", StringComparison.OrdinalIgnoreCase));

            if (finalPackage == null)
            {
                AppLogger.Log("[INSTALL] Package not found. Installation skipped.");
                SetState(DownloadState.Error);
                return;
            }

            try
            {
                SetState(DownloadState.Installing);

                var communityPath = new CommunityFolderService().GetCommunityPath();
                if (string.IsNullOrWhiteSpace(communityPath))
                    throw new Exception("Community folder not found.");

                var installer = new InstallerService();

                AppLogger.Log($"[INSTALL] Extracting: {finalPackage}");
                var extractedPath = installer.ExtractPackage(finalPackage);

                AppLogger.Log("[INSTALL] Installing scenery...");
                installer.InstallFromExtracted(extractedPath, communityPath);

                CleanupSceneriesRoot();

                _sessionService.Clear();
                SetState(DownloadState.Completed);

                AppLogger.Log("[INSTALL] Installation completed successfully.");
            }
            catch (Exception ex)
            {
                AppLogger.LogError("[INSTALL] Installation failed", ex);
                SetState(DownloadState.Error);

                MessageBox.Show(
                    "The scenery was downloaded but could not be installed.\n\n" +
                    "You can retry installation from History.",
                    "Installation failed",
                    MessageBoxButton.OK,
                    MessageBoxImage.Warning
                );
            }
        }

    }
}
