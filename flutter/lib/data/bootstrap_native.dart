import 'package:path_provider/path_provider.dart';
import 'native_repository.dart';
import 'repository.dart';

Future<MailRepository> openRepository() async {
  final directory = await getApplicationSupportDirectory();
  return NativeRepository.open('${directory.path}/mail.sqlite3');
}
