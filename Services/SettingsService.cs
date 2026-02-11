using System;
using System.IO;
using System.Text.Json;

namespace SceneryAddonsBrowser.Services
{
    public class SettingsService
    {
        private readonly string _filePath;

        public SettingsService()
        {
            var baseDir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments),
                "SceneryAddonsBrowser");

            Directory.CreateDirectory(baseDir);
            _filePath = Path.Combine(baseDir, "settings.json");
        }

        public AppSettings Load()
        {
            if (!File.Exists(_filePath))
                return new AppSettings();

            var json = File.ReadAllText(_filePath);
            return JsonSerializer.Deserialize<AppSettings>(json) ?? new AppSettings();
        }

        public void Save(AppSettings settings)
        {
            File.WriteAllText(
                _filePath,
                JsonSerializer.Serialize(settings, new JsonSerializerOptions { WriteIndented = true }));
        }
    }

    public class AppSettings
    {
    }
}
