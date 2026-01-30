using SceneryAddonsBrowser.Models;
using System;
using System.Collections.Generic;
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

        public void Add(DownloadHistoryItem item)
        {
            var list = Load();
            list.Add(item);

            File.WriteAllText(
                _filePath,
                JsonSerializer.Serialize(list, new JsonSerializerOptions { WriteIndented = true }));
        }

        public List<DownloadHistoryItem> Load()
        {
            if (!File.Exists(_filePath))
                return new List<DownloadHistoryItem>();

            var json = File.ReadAllText(_filePath);
            return JsonSerializer.Deserialize<List<DownloadHistoryItem>>(json)
                   ?? new List<DownloadHistoryItem>();
        }
    }
}
