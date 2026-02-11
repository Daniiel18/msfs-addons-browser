using SceneryAddonsBrowser.Models;
using System.Linq;
using System.Windows;
using System.Windows.Controls;

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
        }

        private void MethodsListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
        {
            SelectButton.IsEnabled = MethodsListView.SelectedItem != null;
        }

        private void Select_Click(object sender, RoutedEventArgs e)
        {
            if (MethodsListView.SelectedItem is not DownloadMethod method)
                return;

            if (method.Type == DownloadType.Mirror)
            {
                var result = System.Windows.MessageBox.Show(
    "This download will open your web browser.\n\nDo you want to continue?",
    "Open external mirror",
    MessageBoxButton.YesNo,
    MessageBoxImage.Question);


                if (result != MessageBoxResult.Yes)
                    return;
            }

            SelectedMethod = method;
            DialogResult = true;
        }

        private void Cancel_Click(object sender, RoutedEventArgs e)
        {
            DialogResult = false;
        }
    }
}
