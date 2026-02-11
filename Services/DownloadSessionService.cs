using SceneryAddonsBrowser.Models;
using System.IO;
using System.Text.Json;

namespace SceneryAddonsBrowser.Services
{
    public class DownloadSessionService
    {
        private static string SessionFile =>
            Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                "SceneryAddonsBrowser",
                "active-download.json");

        public void Save(DownloadSession session)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(SessionFile)!);
            var json = JsonSerializer.Serialize(session, new JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText(SessionFile, json);
        }

        public DownloadSession? Load()
        {
            if (!File.Exists(SessionFile))
                return null;

            var json = File.ReadAllText(SessionFile);
            return JsonSerializer.Deserialize<DownloadSession>(json);
        }

        public void Clear()
        {
            if (File.Exists(SessionFile))
                File.Delete(SessionFile);
        }
    }
}
