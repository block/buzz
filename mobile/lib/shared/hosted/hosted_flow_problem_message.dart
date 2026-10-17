import 'hosted_flow.dart';

String hostedFlowProblemMessage(
  HostedFlowProblem? problem,
) => switch (problem) {
  HostedFlowProblem.communityCreationRejected =>
    'This community could not be created. Leave setup to choose another address, or check your hosted community limit in Builderlab.',
  HostedFlowProblem.unsupportedEnvironment =>
    'This community needs a different or unsupported Builderlab environment. Use an app build configured for this community.',
  HostedFlowProblem.accountSetupRequired =>
    'This account is not eligible for Buzz yet.',
  HostedFlowProblem.invitePolicyRequired =>
    'The community policy needs to be accepted again. Try again to review it.',
  HostedFlowProblem.inviteExpired =>
    'This invite has expired. Ask for a new invite link.',
  HostedFlowProblem.inviteExhausted =>
    'This invite has no uses left. Ask for a new invite link.',
  HostedFlowProblem.inviteInvalid =>
    'This invite is not valid. Ask for a new invite link.',
  HostedFlowProblem.denied => 'This account does not have access.',
  HostedFlowProblem.differentAccount =>
    'Your browser signed in to a different account. Sign in to your original account on the web, then reopen setup. Your saved account and keys have been kept.',
  HostedFlowProblem.identityUnreadable =>
    'Your saved identity could not be read safely. Leave setup and recover access to this device’s secure storage before continuing.',
  HostedFlowProblem.backupUnavailable =>
    'Secure backup is not supported on this device yet. Your identity has been kept; joining requires a confirmed backup.',
  HostedFlowProblem.requestRejected =>
    'The account service could not accept this setup request. Your saved account and identity have been kept.',
  HostedFlowProblem.invalidResponse =>
    'The account service returned a response this app cannot safely use. Setup has stopped.',
  HostedFlowProblem.identityMismatch =>
    'This account uses a different identity. Your saved identity has been kept.',
  HostedFlowProblem.backupRequired =>
    'Your identity needs a confirmed cloud backup before joining. Your saved key has been kept.',
  HostedFlowProblem.bindingRequired =>
    'Connect your identity from the account setup page first.',
  HostedFlowProblem.identityUnavailable =>
    'We could not safely recover an identity for this account. Your saved keys have been kept.',
  _ => 'We could not finish connecting. You can try again.',
};
