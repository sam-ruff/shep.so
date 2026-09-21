import 'package:flutter/material.dart';
import '../data/repository.dart';
import '../model/workspace.dart';

class CalDavConnectionsCard extends StatefulWidget {
  const CalDavConnectionsCard({super.key, required this.workspace});
  final Workspace workspace;

  @override
  State<CalDavConnectionsCard> createState() => _CalDavConnectionsCardState();
}

class _CalDavConnectionsCardState extends State<CalDavConnectionsCard> {
  final url = TextEditingController();
  final username = TextEditingController();
  final password = TextEditingController();
  bool expanded = false;
  CalDavAttempt? retryAttempt;
  CalDavConnection? observedConnection;
  String? formError;

  @override
  void initState() {
    super.initState();
    widget.workspace.refreshCalDavConnections(resume: true);
  }

  @override
  void dispose() {
    url.dispose();
    username.dispose();
    password.dispose();
    super.dispose();
  }

  Future<void> connect() async {
    final uri = Uri.tryParse(url.text.trim());
    final host = uri?.host ?? '';
    if (host.isEmpty || username.text.trim().isEmpty || password.text.isEmpty) {
      setState(
        () => formError = 'Enter the calendar URL, username and password.',
      );
      return;
    }
    final retry = retryAttempt;
    late final bool admitted;
    try {
      if (retry == null) {
        admitted = await widget.workspace.connectCalDav(
          connectionId: observedConnection?.id,
          url: url.text.trim(),
          username: username.text.trim(),
          password: password.text,
          observed: observedConnection,
        );
      } else {
        await widget.workspace.retryCalDavWithPassword(retry, password.text);
        admitted = true;
      }
    } on Object catch (error) {
      if (!mounted) return;
      setState(() => formError = error.toString());
      return;
    }
    if (!mounted) return;
    if (!admitted) {
      setState(() => formError = widget.workspace.error);
      return;
    }
    password.clear();
    setState(() {
      expanded = false;
      retryAttempt = null;
      observedConnection = null;
      formError = null;
    });
  }

  void retry(CalDavAttempt attempt) {
    url.text = attempt.url;
    username.text = attempt.username;
    setState(() {
      retryAttempt = attempt;
      observedConnection = null;
      expanded = true;
      formError = null;
    });
  }

  void reconnect(CalDavConnection connection) {
    url.text = connection.url;
    username.text = connection.username;
    setState(() {
      retryAttempt = null;
      observedConnection = connection;
      expanded = true;
      formError = null;
    });
  }

  @override
  Widget build(BuildContext context) => Card(
    child: Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              const Expanded(
                child: Text(
                  'CalDAV calendars',
                  style: TextStyle(fontWeight: FontWeight.w600),
                ),
              ),
              TextButton(
                onPressed: () => setState(() {
                  expanded = !expanded;
                  if (expanded) {
                    retryAttempt = null;
                    observedConnection = null;
                    url.clear();
                    username.clear();
                    password.clear();
                    formError = null;
                  }
                }),
                child: Text(expanded ? 'Close' : 'Add calendar'),
              ),
            ],
          ),
          if (expanded) ...[
            TextField(
              controller: url,
              keyboardType: TextInputType.url,
              decoration: const InputDecoration(labelText: 'Calendar URL'),
            ),
            TextField(
              controller: username,
              decoration: const InputDecoration(labelText: 'Username'),
            ),
            TextField(
              controller: password,
              obscureText: true,
              decoration: const InputDecoration(labelText: 'Password'),
              onSubmitted: (_) => connect(),
            ),
            const SizedBox(height: 12),
            FilledButton(onPressed: connect, child: const Text('Connect')),
            if (formError != null) Text(formError!),
          ],
          if (widget.workspace.calDavCleanupError case final cleanupError?)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: const Text('Credential cleanup waiting'),
              subtitle: Text(cleanupError),
              trailing: TextButton(
                onPressed: widget.workspace.retryCalDavCleanup,
                child: const Text('Retry cleanup'),
              ),
            ),
          for (final connection in widget.workspace.calDavConnections)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(connection.username),
              subtitle: Text(connection.url),
              trailing: Wrap(
                children: [
                  TextButton(
                    onPressed: () => reconnect(connection),
                    child: const Text('Reconnect'),
                  ),
                  TextButton(
                    onPressed: () => widget.workspace.removeCalDav(connection),
                    child: const Text('Remove'),
                  ),
                ],
              ),
            ),
          for (final attempt in widget.workspace.calDavAttempts.where(
            (attempt) => const {
              'prepared',
              'probing',
              'waiting',
            }.contains(attempt.status),
          ))
            _AttemptRow(
              attempt: attempt,
              workspace: widget.workspace,
              onRetry: () => retry(attempt),
            ),
        ],
      ),
    ),
  );
}

class _AttemptRow extends StatelessWidget {
  const _AttemptRow({
    required this.attempt,
    required this.workspace,
    required this.onRetry,
  });
  final CalDavAttempt attempt;
  final Workspace workspace;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) => ListTile(
    contentPadding: EdgeInsets.zero,
    title: Text(
      attempt.status == 'probing' ? 'Checking calendar' : 'Calendar waiting',
    ),
    subtitle: Text(attempt.error ?? 'Saved on this device'),
    trailing: Wrap(
      children: [
        if (attempt.status != 'probing')
          TextButton(onPressed: onRetry, child: const Text('Enter password')),
        TextButton(
          onPressed: () => workspace.cancelCalDav(attempt),
          child: const Text('Cancel'),
        ),
      ],
    ),
  );
}
