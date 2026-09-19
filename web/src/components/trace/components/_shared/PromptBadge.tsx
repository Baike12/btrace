import { Badge } from "@/src/components/ui/badge";
import { api } from "@/src/utils/api";

export const PromptBadge = (props: { promptId: string; projectId: string }) => {
  const prompt = api.prompts.byId.useQuery({
    id: props.promptId,
    projectId: props.projectId,
  });

  if (prompt.isLoading || !prompt.data) return null;

  return (
    <Badge variant="tertiary" className="inline-flex">
      <span className="truncate">
        Prompt: {prompt.data.name}
        {" - v"}
        {prompt.data.version}
      </span>
    </Badge>
  );
};
