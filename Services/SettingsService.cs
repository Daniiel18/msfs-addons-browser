using System.IO;
using System.Text.Json;

public class SettingsService
{
    private readonly string _filePath;

    public SettingsService()
    {
        // ❌ NO usar UserStorage aquí
        var baseDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "SceneryAddonsBrowser"
        );

        Directory.CreateDirectory(baseDir);
        _filePath = Path.Combine(baseDir, "settings.json");
    }

    public AppSettings Load()
    {
        if (!File.Exists(_filePath))
            return new AppSettings();

        return JsonSerializer.Deserialize<AppSettings>(
            File.ReadAllText(_filePath)
        ) ?? new AppSettings();
    }

    public void Save(AppSettings settings)
    {
        File.WriteAllText(
            _filePath,
            JsonSerializer.Serialize(settings, new JsonSerializerOptions
            {
                WriteIndented = true
            })
        );
    }
}

public class AppSettings
{
    public string? DownloadRoot { get; set; }

    public string? IgnoredUpdateVersion  { get; set; }
}

public static class UserStorage
    {
        private static string? _rootPath;

        public static string RootPath
        {
            get
            {
                if (string.IsNullOrWhiteSpace(_rootPath))
                    throw new InvalidOperationException("User storage root not initialized");

                return _rootPath;
            }
        }

        public static void SetRoot(string baseFolder)
        {
            if (string.IsNullOrWhiteSpace(baseFolder))
                throw new ArgumentException("Base folder cannot be empty");

            _rootPath = Path.Combine(baseFolder, "SceneryAddonsBrowser");
            Directory.CreateDirectory(_rootPath);
        }

    }
