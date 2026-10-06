import '../model/preferences_search.dart';
import 'repository.dart';
import 'native_repository.dart';
import 'preferences_search_native.dart';

PreferenceSearchMatcher preferenceSearchMatcher(MailRepository repository) {
  if (repository is NativeRepository) {
    return NativePreferenceSearchMatcher(repository);
  }
  return const PreviewPreferenceSearchMatcher();
}
