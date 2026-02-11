using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows.Media;
using Brush = System.Windows.Media.Brush;
using Brushes = System.Windows.Media.Brushes;

namespace SceneryAddonsBrowser.Models
{
    public class DownloadStatus : INotifyPropertyChanged
    {
        private double _percent;
        private string _text = "Ready";
        private Brush _barColor = Brushes.Gray;
        private DownloadState _state;

        public double Percent
        {
            get => _percent;
            set { _percent = value; OnChanged(); }
        }

        public string Text
        {
            get => _text;
            set { _text = value; OnChanged(); }
        }

        public Brush BarColor
        {
            get => _barColor;
            set { _barColor = value; OnChanged(); }
        }

        public DownloadState State
        {
            get => _state;
            set { _state = value; OnChanged(); }
        }

        public event PropertyChangedEventHandler? PropertyChanged;

        private void OnChanged([CallerMemberName] string? name = null)
            => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
    }
}
