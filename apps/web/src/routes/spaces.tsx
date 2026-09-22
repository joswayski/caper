import { createFileRoute } from "@tanstack/react-router";
import Spaces from "../spaces/Spaces";

export const Route = createFileRoute("/spaces")({ component: Spaces });
