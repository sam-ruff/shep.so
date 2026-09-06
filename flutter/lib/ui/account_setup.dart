import 'dart:async';
import 'package:flutter/material.dart';
import '../data/accounts.dart';
import '../model/workspace.dart';

class AccountSetup extends StatefulWidget {
  const AccountSetup({super.key, required this.workspace, this.account});
  final Workspace workspace;
  final MailAccount? account;
  @override
  State<AccountSetup> createState() => _AccountSetupState();
}

class _AccountSetupState extends State<AccountSetup> {
  final form = GlobalKey<FormState>();
  late final name = TextEditingController(text: widget.account?.name);
  late final email = TextEditingController(text: widget.account?.email);
  late final host = TextEditingController(text: widget.account?.host);
  late final port = TextEditingController(
    text: '${widget.account?.port ?? 993}',
  );
  late final username = TextEditingController(text: widget.account?.username);
  late final smtpHost = TextEditingController(text: widget.account?.smtpHost);
  late final smtpPort = TextEditingController(
    text: '${widget.account?.smtpPort ?? 465}',
  );
  late final smtpUsername = TextEditingController(
    text: widget.account?.smtpUsername,
  );
  final password = TextEditingController(),
      smtpPassword = TextEditingController();
  late String sentCopy = widget.account?.sentCopy ?? 'Automatic';
  late final sentFolder = TextEditingController(
    text: widget.account?.sentFolder,
  );
  late String protocol = widget.account?.protocol ?? 'Imap';
  late String security = widget.account?.security ?? 'Tls';
  late String authentication = widget.account?.authentication ?? 'Password';
  late String smtpSecurity = widget.account?.smtpSecurity ?? 'Tls';
  late String smtpAuthentication =
      widget.account?.smtpAuthentication ?? 'Automatic';
  late bool separate = widget.account?.separatePassword ?? false;
  late final id =
      widget.account?.id ?? 'account-${DateTime.now().microsecondsSinceEpoch}';
  bool busy = false;
  String? error;
  bool get reconnect => widget.account != null;

  @override
  void dispose() {
    for (final c in [
      name,
      email,
      host,
      port,
      username,
      smtpHost,
      smtpPort,
      smtpUsername,
      password,
      smtpPassword,
      sentFolder,
    ]) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> connect() async {
    if (!form.currentState!.validate()) return;
    FocusScope.of(context).unfocus();
    setState(() {
      busy = true;
      error = null;
    });
    final account =
        widget.account ??
        MailAccount(
          id: id,
          name: name.text.trim(),
          email: email.text.trim(),
          host: host.text.trim(),
          port: int.parse(port.text),
          username: username.text.trim(),
          protocol: protocol,
          security: security,
          authentication: authentication,
          smtpHost: smtpHost.text.trim(),
          smtpPort: int.parse(smtpPort.text),
          smtpUsername: smtpUsername.text.trim(),
          smtpSecurity: smtpSecurity,
          smtpAuthentication: smtpAuthentication,
          separatePassword: separate,
          sentCopy: protocol == 'Pop3' ? 'LocalOnly' : sentCopy,
          sentFolder: sentFolder.text.trim(),
        );
    try {
      await widget.workspace.accountRepository!.connect(
        account,
        password.text,
        separate ? smtpPassword.text : password.text,
      );
      if (!mounted) return;
      // Cached workspace is usable while the first provider sync runs.
      unawaited(widget.workspace.refresh());
      Navigator.pop(context);
    } catch (e) {
      if (mounted) {
        setState(() {
          error = '$e';
          busy = false;
        });
      }
    }
  }

  Widget field(
    TextEditingController controller,
    String label, {
    bool secret = false,
    bool number = false,
    bool required = true,
  }) => Padding(
    padding: const EdgeInsets.only(bottom: 14),
    child: TextFormField(
      controller: controller,
      enabled: !busy && (!reconnect || secret),
      obscureText: secret,
      autocorrect: !secret,
      enableSuggestions: !secret,
      keyboardType: number
          ? TextInputType.number
          : secret
          ? TextInputType.visiblePassword
          : TextInputType.text,
      decoration: InputDecoration(labelText: label),
      validator: (value) {
        if (required && (value == null || value.trim().isEmpty)) {
          return 'Enter ${label.toLowerCase()}.';
        }
        if (number &&
            ((int.tryParse(value ?? '') ?? 0) < 1 ||
                (int.tryParse(value ?? '') ?? 0) > 65535)) {
          return 'Use a port from 1 to 65535.';
        }
        return null;
      },
    ),
  );

  Widget choice(
    String label,
    String value,
    Map<String, String> choices,
    void Function(String) change,
  ) => Padding(
    padding: const EdgeInsets.only(bottom: 14),
    child: DropdownButtonFormField<String>(
      initialValue: value,
      isExpanded: true,
      decoration: InputDecoration(labelText: label),
      items: choices.entries
          .map((e) => DropdownMenuItem(value: e.key, child: Text(e.value)))
          .toList(),
      onChanged: busy || reconnect
          ? null
          : (v) {
              if (v != null) setState(() => change(v));
            },
    ),
  );

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: Text(reconnect ? 'Reconnect account' : 'Add mail account'),
    ),
    bottomNavigationBar: SafeArea(
      top: false,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (error != null)
            Material(
              color: Theme.of(context).colorScheme.errorContainer,
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Text(
                  error!,
                  style: TextStyle(
                    color: Theme.of(context).colorScheme.onErrorContainer,
                  ),
                ),
              ),
            ),
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 12, 20, 16),
            child: SizedBox(
              width: double.infinity,
              child: FilledButton.icon(
                onPressed: busy ? null : connect,
                icon: busy
                    ? const SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.link),
                label: Text(
                  busy
                      ? 'Checking connections…'
                      : reconnect
                      ? 'Reconnect'
                      : 'Connect account',
                ),
              ),
            ),
          ),
        ],
      ),
    ),
    body: Form(
      key: form,
      child: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          if (reconnect)
            Padding(
              padding: const EdgeInsets.only(bottom: 16),
              child: Text(widget.account!.email),
            ),
          if (!reconnect) ...[
            field(name, 'Account name'),
            field(email, 'Email address'),
            const Text('Incoming mail'),
            const SizedBox(height: 16),
            choice(
              'Protocol',
              protocol,
              const {'Imap': 'IMAP', 'Pop3': 'POP3'},
              (v) {
                protocol = v;
                port.text = security == 'Tls'
                    ? (v == 'Imap' ? '993' : '995')
                    : (v == 'Imap' ? '143' : '110');
              },
            ),
            field(host, 'Incoming hostname'),
            field(port, 'Incoming port', number: true),
            choice(
              'Incoming security',
              security,
              const {'Tls': 'SSL/TLS', 'StartTls': 'STARTTLS'},
              (v) {
                security = v;
                port.text = v == 'Tls'
                    ? (protocol == 'Imap' ? '993' : '995')
                    : (protocol == 'Imap' ? '143' : '110');
              },
            ),
            choice('Incoming authentication', authentication, const {
              'Password': 'Normal password',
              'Plain': 'SASL PLAIN',
            }, (v) => authentication = v),
            field(username, 'Incoming username'),
          ],
          field(password, 'Incoming password', secret: true),
          if (!reconnect) ...[
            const SizedBox(height: 12),
            const Text('Outgoing mail'),
            const SizedBox(height: 16),
            field(smtpHost, 'SMTP hostname'),
            field(smtpPort, 'SMTP port', number: true),
            choice(
              'SMTP security',
              smtpSecurity,
              const {'Tls': 'SSL/TLS', 'StartTls': 'STARTTLS'},
              (v) {
                smtpSecurity = v;
                smtpPort.text = v == 'Tls' ? '465' : '587';
              },
            ),
            choice(
              'SMTP authentication',
              smtpAuthentication,
              const {
                'Automatic': 'Automatic',
                'Plain': 'PLAIN',
                'Login': 'LOGIN',
                'None': 'No authentication',
              },
              (v) => smtpAuthentication = v,
            ),
            field(smtpUsername, 'SMTP username (optional)', required: false),
            SwitchListTile(
              contentPadding: EdgeInsets.zero,
              title: const Text('Separate SMTP password'),
              value: separate,
              onChanged: busy ? null : (v) => setState(() => separate = v),
            ),
          ],
          if (separate) field(smtpPassword, 'SMTP password', secret: true),
          const Text(
            'Passwords are kept in this device’s secure credential storage. Connecting checks both servers without sending mail.',
          ),
          const SizedBox(height: 12),
          if (!reconnect && protocol == 'Imap') ...[
            choice('Sent copies', sentCopy, const {
              'Automatic': 'Save a copy on the mail server',
              'ServerManaged': 'My server saves Sent automatically',
              'LocalOnly': 'Keep Sent on this device',
            }, (v) => sentCopy = v),
            if (sentCopy != 'LocalOnly')
              field(sentFolder, 'Sent folder (optional)', required: false),
            const Text(
              'Leave the folder empty to use the server’s Sent folder. Unconfirmed uploads remain in Outbox for review.',
            ),
          ],
          if (protocol == 'Pop3')
            const Text('POP3 keeps Sent copies on this device.'),
          const SizedBox(height: 20),
        ],
      ),
    ),
  );
}
