using System.ComponentModel;
using System.Runtime.CompilerServices;
using Brush = System.Windows.Media.Brush;
using Brushes = System.Windows.Media.Brushes;

namespace SceneryAddonsBrowser.Models
{
    public class DownloadStatus : INotifyPropertyChanged
    {
        private Brush _barColor = Brushes.Gray;

        public event PropertyChangedEventHandler? PropertyChanged;

    }
}
