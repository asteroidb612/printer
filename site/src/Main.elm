module Main exposing (main)

import Browser exposing (document)
import Debug
import Html exposing (a, div, text)
import Html.Attributes exposing (href)
import Http
import Iso8601
import Json.Decode as D
import List exposing (map)
import Time exposing (Posix)


main =
    document
        { init = init
        , view = view
        , update = update
        , subscriptions = \_ -> Sub.none
        }


init : () -> ( Model, Cmd Msg )
init flags =
    ( { games = Nothing }, fetchGames )


type alias Game =
    { name : String, start : Posix, end : Posix, events : List Posix }


type alias Model =
    { games : Maybe (List Game) }


type Msg
    = GotGames (Result Http.Error (List Game))
    | ButtonPressed


view model =
    { title = "Ludi"
    , body = [ text "Hello World" ]
    }


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotGames result ->
            case Debug.log "result" result of
                Ok games ->
                    ( { model | games = Just (Debug.log "games" games) }, Cmd.none )

                _ ->
                    Debug.todo "What should I do if I can't parse games from the server?"

        ButtonPressed ->
            ( model, fetchGames )


fetchGames : Cmd Msg
fetchGames =
    Http.get
        { url = "/read_game_file"
        , expect = Http.expectJson GotGames (D.field "games" (D.list decodeGame))
        }


decodeGame =
    D.map4 Game
        (D.field "name" D.string)
        (D.field "start" Iso8601.decoder)
        (D.field "end" Iso8601.decoder)
        (D.field "events" (D.list Iso8601.decoder))
