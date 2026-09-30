import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";

type ProfileAvatarUploadPreviewProps = {
  avatarUrl: string;
  label: string;
  testId: string;
};

export function ProfileAvatarUploadPreview({
  avatarUrl,
  label,
  testId,
}: ProfileAvatarUploadPreviewProps) {
  return (
    <ProfileAvatar
      avatarUrl={avatarUrl}
      className="h-20 w-20 text-xl"
      imageClassName="object-cover"
      label={label}
      testId={testId}
    />
  );
}
