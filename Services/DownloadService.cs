using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using System.Diagnostics;
using System.IO;
using System.Net.Http;
using System.Windows;
using Microsoft.Win32;



namespace SceneryAddonsBrowser.Services
{
    public class DownloadService
    {
        private readonly HttpClient _httpClient;
        private readonly HistoryService _historyService = new HistoryService();

        public DownloadService()
        {
            _httpClient = new HttpClient(
                new HttpClientHandler
                {
                    AllowAutoRedirect = false
                });
        }

        public async Task StartDownloadAsync(DownloadMethod method)
        {
            AppLogger.Log("=== StartDownloadAsync ===");

            if (method == null)
            {
                AppLogger.Log("DownloadMethod is null");
                return;
            }

            AppLogger.Log($"Method name: {method.Name}");
            AppLogger.Log($"Method URL: {method.Url}");
            AppLogger.Log($"Method type: {method.Type}");

            // ⬇️ AQUÍ
            if (method.Scenario != null)
            {
                _historyService.Add(new DownloadHistoryItem
                {
                    Icao = method.Scenario.Icao,
                    ScenarioName = method.Scenario.Name,
                    Developer = method.Scenario.Developer,
                    Method = method.Name,
                    DownloadDate = DateTime.Now
                });
            }

            try
            {
                if (method.Type == DownloadType.Torrent)
                {
                    AppLogger.Log("Torrent selected → resolving automatically");
                    await ResolveTorrentAsync(method.Url);
                }
                else
                {
                    AppLogger.Log("External mirror selected → opening system browser");

                    Process.Start(new ProcessStartInfo
                    {
                        FileName = method.Url,
                        UseShellExecute = true
                    });
                }
            }
            catch (Exception ex)
            {
                AppLogger.LogError("Download failed", ex);

                MessageBox.Show(
                    "This download could not be started.\n\n" +
                    "Please check the log for details.",
                    "Download error",
                    MessageBoxButton.OK,
                    MessageBoxImage.Warning);
            }
        }

        private async Task ResolveTorrentAsync(string getUrl)
        {
            AppLogger.Log($"ResolveTorrentAsync URL: {getUrl}");

            string html;

            try
            {
                html = await _httpClient.GetStringAsync(getUrl);
                AppLogger.Log($"HTML length: {html.Length}");
            }
            catch (Exception ex)
            {
                AppLogger.LogError("HTTP GET failed", ex);
                throw;
            }

            int magnetIndex = html.IndexOf("magnet:?", StringComparison.OrdinalIgnoreCase);

            if (magnetIndex < 0)
            {
                AppLogger.Log("Magnet link NOT found in HTML");
                SaveDebugHtml(html);
                throw new Exception("Magnet link not found.");
            }

            AppLogger.Log($"Magnet found at index {magnetIndex}");

            string magnet = ExtractMagnet(html, magnetIndex);

            AppLogger.Log($"Extracted magnet: {magnet}");

            if (!OpenMagnetWithSystemHandler(magnet))
            {
                MessageBox.Show(
                    "No torrent client is properly configured on this system.\n\n" +
                    "Please install or configure a torrent client (uTorrent, qBittorrent, etc.).",
                    "Torrent client not found",
                    MessageBoxButton.OK,
                    MessageBoxImage.Warning);
            }
        }

        private string ExtractMagnet(string html, int startIndex)
        {
            var magnet = html.Substring(startIndex);

            int endIndex = magnet.IndexOf('"');
            if (endIndex < 0)
                endIndex = magnet.IndexOf('\'');

            if (endIndex > 0)
                magnet = magnet.Substring(0, endIndex);

            return magnet.Trim();
        }

        private void SaveDebugHtml(string html)
        {
            try
            {
                string debugDir =
                    Path.Combine(
                        Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments),
                        "SceneryAddonsBrowser",
                        "logs",
                        "html-dumps"
                    );

                Directory.CreateDirectory(debugDir);

                string filePath =
                    Path.Combine(debugDir, $"torrent_debug_{DateTime.Now:yyyyMMdd_HHmmss}.html");

                File.WriteAllText(filePath, html);

                AppLogger.Log($"HTML dumped to: {filePath}");
            }
            catch
            {
                // no-op
            }
        }

        private bool OpenMagnetWithSystemHandler(string magnet)
        {
            try
            {
                using var key = Registry.ClassesRoot
                    .OpenSubKey(@"magnet\shell\open\command");

                if (key == null)
                {
                    AppLogger.Log("No magnet handler found in registry.");
                    return false;
                }

                string command = key.GetValue(null)?.ToString();

                if (string.IsNullOrWhiteSpace(command))
                {
                    AppLogger.Log("Magnet handler command is empty.");
                    return false;
                }

                AppLogger.Log($"Magnet handler command: {command}");

                command = command.Replace("%1", magnet).Trim();

                int exeIndex = command.IndexOf(".exe", StringComparison.OrdinalIgnoreCase);
                if (exeIndex < 0)
                {
                    AppLogger.Log("No .exe found in handler command.");
                    return false;
                }

                exeIndex += 4;

                string exePath = command.Substring(0, exeIndex).Trim().Trim('"');
                string arguments = command.Substring(exeIndex).Trim();

                AppLogger.Log($"Final EXE path: {exePath}");
                AppLogger.Log($"Final arguments: {arguments}");

                if (!File.Exists(exePath))
                {
                    AppLogger.Log("Final EXE does not exist on disk.");
                    return false;
                }

                Process.Start(new ProcessStartInfo
                {
                    FileName = exePath,
                    Arguments = arguments,
                    UseShellExecute = false
                });

                return true;
            }
            catch (Exception ex)
            {
                AppLogger.LogError("Failed to launch magnet handler", ex);
                return false;
            }
        }
    }
}
