namespace SceneryAddonsBrowser.Models
{
    public class DownloadSession
    {
        public string ScenarioId { get; set; } = "";
        public string MagnetUri { get; set; } = "";
        public string DataPath { get; set; } = "";
        public bool IsCompleted { get; set; }
    }
}
