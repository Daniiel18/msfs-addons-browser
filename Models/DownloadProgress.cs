namespace SceneryAddonsBrowser.Models
{
    public class DownloadProgress
    {
        public double Percent { get; set; }
        public double SpeedMbps { get; set; }
        public TimeSpan Eta { get; set; }
        public string Status { get; set; } = "";
    }
}
