using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace SceneryAddonsBrowser.Models
{
    public class Scenario : INotifyPropertyChanged
    {
        public event PropertyChangedEventHandler? PropertyChanged;

        private void Notify([CallerMemberName] string? prop = null)
            => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(prop));

        public string Icao { get; set; } = "";
        public string Name { get; set; } = "";
        public string Developer { get; set; } = "";
        public string Simulator { get; set; } = "";
        public string Version { get; set; } = "";
        public string SourcePageUrl { get; set; } = "";

        public List<DownloadMethod> DownloadMethods { get; set; } = new();

        // ================= GSX =================

        private string _gsxText = "GSX Profile — Checking…";
        public string GsxText
        {
            get => _gsxText;
            set
            {
                _gsxText = value;
                Notify();
            }
        }

        private string? _gsxUrl;
        public string? GsxUrl
        {
            get => _gsxUrl;
            set
            {
                _gsxUrl = value;
                Notify();
            }
        }
    }
}
