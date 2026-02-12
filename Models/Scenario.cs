namespace SceneryAddonsBrowser.Models
{
    public class Scenario
    {
        public string Icao { get; set; }

        public string Name { get; set; }

        public string Developer { get; set; }

        public string Simulator { get; set; }

        public string Version { get; set; }

        public string SourcePageUrl { get; set; }

        public List<DownloadMethod> DownloadMethods { get; set; }

        public Scenario()
        {
            DownloadMethods = new List<DownloadMethod>();
        }
    }
}
