using SceneryAddonsBrowser.Services;
using System.Windows;
using System.Windows.Input;

namespace SceneryAddonsBrowser
{
    public partial class HistoryDialog : Window
    {
        private readonly HistoryService _historyService = new();
        private readonly InstallerService _installerService = new();
        private readonly CommunityFolderService _communityService = new();

        private List<DownloadHistoryItem> _items;

        public HistoryDialog()
        {
            InitializeComponent();
            _items = _historyService.Load();
            DataContext = _items;

            _items = _historyService.Load();

            var communityPath = _communityService.GetCommunityPath();
        }

        private void Close_Click(object sender, RoutedEventArgs e)
        {
            Close();
        }

        private void Header_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
        {
            DragMove();
        }
    }
}
