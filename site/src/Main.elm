module Main exposing (main)

import Html exposing (a, div, text)
import Html.Attributes exposing (href)
import List exposing (map)

games = ["wake and code"]
main = div [] <| List.map link_from_game games

link_from_game game_name = a [href game_name] [text game_name]
