using SceneryAddonsBrowser.Models;
using System.Windows;
using System.Windows.Controls;
using SceneryAddonsBrowser.Logging;

namespace SceneryAddonsBrowser
{
    public partial class DownloadDialog : Window
    {
        public DownloadMethod? SelectedMethod { get; private set; }
        public string DialogTitle { get; }

        public DownloadDialog(Scenario scenario)
        {
            InitializeComponent();

            DialogTitle = $"{scenario.Developer} – {scenario.Name}";
            DataContext = this;

            // Torrent primero
            var ordered = scenario.DownloadMethods
                .OrderByDescending(m => m.Type == DownloadType.Torrent)
                .ToList();

            MethodsListView.ItemsSource = ordered;

            AppLogger.Log($"UI: Download dialog opened for scenario '{scenario.Developer} - {scenario.Name}'");
        }

        private void MethodsListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
        {
            SelectButton.IsEnabled = MethodsListView.SelectedItem != null;

            if (MethodsListView.SelectedItem is DownloadMethod method)
            {
                AppLogger.Log($"UI: Download method selected in dialog: {method.Name} ({method.Type})");
            }
        }

        private void Select_Click(object sender, RoutedEventArgs e)
        {
            if (MethodsListView.SelectedItem is not DownloadMethod method)
                return;

            if (method.Type == DownloadType.Mirror)
            {
                AppLogger.Log("UI: User chose mirror method; prompting confirmation to open browser");
                var result = System.Windows.MessageBox.Show(
                    "This download will open your web browser.\n\nDo you want to continue?",
                    "Open external mirror",
                    MessageBoxButton.YesNo,
                    MessageBoxImage.Question);


                if (result != MessageBoxResult.Yes)
                {
                    AppLogger.Log("UI: User canceled mirror open confirmation");
                    return;
                }
            }

            SelectedMethod = method;
            DialogResult = true;

            AppLogger.Log($"UI: Download dialog confirmed. Selected method: {method.Name} ({method.Type})");
        }

        private void Cancel_Click(object sender, RoutedEventArgs e)
        {
            DialogResult = false;
            AppLogger.Log("UI: Download dialog canceled by user");
        }
    }
}
