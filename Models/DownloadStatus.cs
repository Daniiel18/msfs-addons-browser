using System.ComponentModel;
using System.Runtime.CompilerServices;
using Brush = System.Windows.Media.Brush;
using Brushes = System.Windows.Media.Brushes;

namespace SceneryAddonsBrowser.Models
{
    public class DownloadStatus : INotifyPropertyChanged
    {
        private string _text = "Ready";
        private Brush _barColor = Brushes.Gray;
        private DownloadState _state;

        public event PropertyChangedEventHandler? PropertyChanged;

        private void OnChanged([CallerMemberName] string? name = null)
            => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
    }
}
