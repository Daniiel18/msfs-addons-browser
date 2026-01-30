namespace SceneryAddonsBrowser.Models
{
    public enum DownloadType
    {
        Torrent,
        Mirror
    }

    public class DownloadMethod
    {
        public string Name { get; set; } = "";
        public string Url { get; set; } = "";
        public DownloadType Type { get; set; }

        // 🔴 AGREGA ESTO
        public Scenario Scenario { get; set; } = null!;
    }
}
