import { type NextApiRequest, type NextApiResponse } from "next";

export async function createNewSsoConfigHandler(
  req: NextApiRequest,
  res: NextApiResponse,
) {
  res.status(501).json({ error: "SSO configuration is not available in this version" });
}
