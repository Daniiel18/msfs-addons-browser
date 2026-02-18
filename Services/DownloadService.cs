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
                    if (_currentDataDir != null && Directory.Exists(_currentDataDir))
                    {

                     Directory.Delete(Path.GetDirectoryName(_currentDataDir)!, true);
                    
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
                string? magnet = null;

                // 🔁 LOOP CONTROLADO PARA RESOLVER MAGNET
                while (magnet == null)
                {
                    SetState(DownloadState.ResolvingMagnet);
                    progress?.Invoke("Resolving magnet...", 0);

                    magnet = await ResolveMagnetAsync(method.Url);

                    if (magnet == null)
                    {
                        AppLogger.Log("[TORRENT] Magnet not ready. Waiting before retry.");

                        SetState(DownloadState.WaitingForMagnet);
                        progress?.Invoke("Preparing torrent… waiting for server", 0);

                        await Task.Delay(TimeSpan.FromSeconds(15));

                        if (CurrentState == DownloadState.Cancelled)
                        {
                            AppLogger.Log("[TORRENT] Cancelled while waiting for magnet.");
                            return;
                        }
                    }
                }

                // ✅ Magnet listo
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

                var scenarioTitle = BuildScenarioDisplayName(method);

                await _torrentService.DownloadFromMagnetAsync(
                    magnet,
                    dataDir,
                    p =>
                    {
                        progress?.Invoke(
                            $"{scenarioTitle}\nDownloading • {p.Percent:F1}% • {p.SpeedMbps:F1} Mbps • ETA {p.Eta:mm\\:ss}",
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
            catch (TaskCanceledException)
            {
                AppLogger.Log("[TORRENT] Download cancelled by user");
                SetState(DownloadState.Cancelled);
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
        public async Task StartDownloadAsync(DownloadMethod method,Action<string, int>? progressCallback = null)
        {
            AppLogger.Log("=== StartDownloadAsync ===");

            if (method?.Scenario == null)
                return;

            // ✅ Registrar en History UNA SOLA VEZ
            _historyService.AddOrUpdate(new DownloadHistoryItem
            {
                Icao = method.Scenario.Icao,
                ScenarioName = method.Scenario.Name,
                Developer = method.Scenario.Developer,
                Method = method.Type.ToString(),
                DownloadDate = DateTime.Now,
                IsInstalled = false,
                AutoInstallPending = method.Type == DownloadType.Torrent
            });

            // 🚀 TORRENT
            if (method.Type == DownloadType.Torrent)
            {
                _torrentQueue.Enqueue(new TorrentJob(method, progressCallback));
                QueueChanged?.Invoke();
                return;
            }

            // 🌐 MIRROR
            var scenarioTitle = BuildScenarioDisplayName(method);

            progressCallback?.Invoke(
                $"{scenarioTitle}\nOpening download page…",
                0
            );

            Process.Start(new ProcessStartInfo
            {
                FileName = method.Url,
                UseShellExecute = true
            });
        }

        private static string BuildScenarioDisplayName(DownloadMethod method)
        {
            if (method?.Scenario == null)
                return "Downloading";

            return $"{method.Scenario.Icao} – {method.Scenario.Name} – {method.Scenario.Developer}";
        }

        // ================= MAGNET =================
        private async Task<string?> ResolveMagnetAsync(string getUrl)
        {
            const int maxAttempts = 10;
            const int delayMs = 2000;

            AppLogger.Log($"Resolving magnet from: {getUrl}");

            for (int attempt = 1; attempt <= maxAttempts; attempt++)
            {
                try
                {
                    AppLogger.Log($"[TORRENT] Resolve attempt {attempt}/{maxAttempts}");

                    var html = await _httpClient.GetStringAsync(getUrl);

                    if (string.IsNullOrWhiteSpace(html) || html.Length < 200)
                        throw new Exception("HTML response too short, magnet not ready.");

                    int magnetIndex = html.IndexOf("magnet:?", StringComparison.OrdinalIgnoreCase);
                    if (magnetIndex < 0)
                        throw new Exception("Magnet link not found in HTML.");

                    var magnet = html.Substring(magnetIndex);

                    int endIndex = magnet.IndexOf('"');
                    if (endIndex < 0)
                        endIndex = magnet.IndexOf('\'');

                    if (endIndex > 0)
                        magnet = magnet.Substring(0, endIndex);

                    magnet = magnet.Trim();

                    AppLogger.Log($"Resolved magnet: {magnet}");
                    return magnet;
                }
                catch (Exception ex)
                {
                    AppLogger.Log($"[TORRENT] Attempt {attempt} failed: {ex.Message}");

                    if (attempt == maxAttempts)
                        break;

                    await Task.Delay(delayMs);
                }
            }

            AppLogger.Log("[TORRENT] Magnet not available after max attempts.");
            return null;
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

                _historyService.AddOrUpdate(new DownloadHistoryItem
                {
                    Icao = session.ScenarioId.Split('_')[0],
                    ScenarioName = session.ScenarioId,
                    Developer = session.ScenarioId.Split('_')[1],
                    Method = "Torrent",
                    DownloadDate = DateTime.Now,
                    PackagePath = null,
                    IsInstalled = true,
                    AutoInstallPending = false
                });

                CleanupSceneriesRoot();

                _sessionService.Clear();
                SetState(DownloadState.Completed);

                AppLogger.Log("[INSTALL] Installation completed successfully.");
            }
            catch (Exception ex)
            {
                AppLogger.LogError("[INSTALL] Installation failed", ex);
                _historyService.AddOrUpdate(new DownloadHistoryItem
                {
                    Icao = session.ScenarioId.Split('_')[0],
                    ScenarioName = session.ScenarioId,
                    Developer = session.ScenarioId.Split('_')[1],
                    Method = "Torrent",
                    DownloadDate = DateTime.Now,
                    PackagePath = session.DataPath,
                    IsInstalled = false,
                    AutoInstallPending = true
                });

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

        public DownloadSession? GetActiveSession()
        {
            return _sessionService.Load();
        }
    }
}
