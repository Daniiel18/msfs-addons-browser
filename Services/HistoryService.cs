using System.IO;
using System.Text.Json;


namespace SceneryAddonsBrowser.Services
{
    public class HistoryService
    {
        private readonly string _filePath;

        public HistoryService()
        {
            var baseDir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments),
                "SceneryAddonsBrowser");

            Directory.CreateDirectory(baseDir);
            _filePath = Path.Combine(baseDir, "history.json");
        }

        public void AddOrUpdate(DownloadHistoryItem item)
        {
            var list = Load();

            var existing = list.FirstOrDefault(x =>
                x.Icao.Equals(item.Icao, StringComparison.OrdinalIgnoreCase) &&
                x.Developer.Equals(item.Developer, StringComparison.OrdinalIgnoreCase));

            if (existing == null)
            {
                list.Add(item);
            }
            else
            {
                // Actualizar SOLO campos que cambian
                existing.Method = item.Method;
                existing.DownloadDate = item.DownloadDate;
                existing.PackagePath = item.PackagePath ?? existing.PackagePath;
                existing.IsInstalled = item.IsInstalled;
                existing.AutoInstallPending = item.AutoInstallPending;
            }

            Save(list);
        }

        public List<DownloadHistoryItem> Load()
        {
            if (!File.Exists(_filePath))
                return new List<DownloadHistoryItem>();

            var json = File.ReadAllText(_filePath);
            return JsonSerializer.Deserialize<List<DownloadHistoryItem>>(json)
                   ?? new List<DownloadHistoryItem>();
        }

        public void Save(List<DownloadHistoryItem> items)
        {
            File.WriteAllText(
                _filePath,
                System.Text.Json.JsonSerializer.Serialize(
                    items,
                    new System.Text.Json.JsonSerializerOptions { WriteIndented = true }
                )
            );
        }
    }
}
