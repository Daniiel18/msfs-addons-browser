using System;

namespace SceneryAddonsBrowser.Models
{
    public class DownloadHistoryItem
    {
        public string Icao { get; set; } = "";
        public string ScenarioName { get; set; } = "";
        public string Developer { get; set; } = "";
        public string Method { get; set; } = "";
        public DateTime DownloadDate { get; set; }
    }
}
